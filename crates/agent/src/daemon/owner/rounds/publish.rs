//! Publishing: pushing the branch of the lead, opening or updating its pull request, and putting the items in review.

use agent_core::config::RepoName;
use agent_core::decide::fix::{Answers, Budget, Progress, Step, fix_step};
use agent_core::decide::publish::{GateHold, GateStep, HarnessGate, Waiting, gate_step};
use agent_core::decide::round::{RoundEvent, RoundPolicy, RoundState, Stage};
use agent_core::decide::wait::Obligation;
use agent_core::history::{CheckpointCause, HistoryEvent};
use std::num::NonZeroU32;

use crate::daemon::pr::description::RequestWithSummary;
use agent_core::ids::{AgentName, BranchName, ItemId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::{EventKind, Failure, Reply};
use agent_core::state::{CommitId, Completion, GateRecord, ItemPhase, PullRequest, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{DelegationStatus, TaskRequest};
use agent_runtime::git::{Git, Revision};
use agent_runtime::github::{NewPullRequest, PullRequestUpdate, Readiness, RemotePullRequest, RepoSlug};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;
use tokio::sync::oneshot;

use crate::daemon::team::Why;

use super::super::teams::Look;
use crate::daemon::pr::{Joining, Publication, Publish, plan_publications, pull_text, returned_delivery};

use super::super::{Owner, OwnerError};
use crate::daemon::merge::count;

use super::checks::{Held, Reviewing};
use super::{Announcement, AutopilotNotice, Lead, PublishError, Pulls};

/// Refreshes only links attributable to the origin used by this publication.
async fn refresh_harness_files(git: &Git, description: &str, harness: &str) -> String {
    let repository = async {
        let origin = git.origin_url().await?;
        Ok::<_, PublishError>(RepoSlug::from_remote_url(&origin)?)
    }
    .await;
    match repository {
        Ok(slug) => {
            let url = format!("https://github.com/{}/{}", slug.owner(), slug.name());
            crate::daemon::checks::unlinked_harness_files_in_repository(description, harness, &url)
        }
        Err(error) => {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                error = %error_chain(&error),
                "cannot determine origin for refreshing file links: text is sent unchanged"
            );
            description.to_owned()
        }
    }
}

/// How many times in a row a lead is sent back because the gate has not passed at the tip of its branch; after that the
/// human is told.
pub(super) const GATE_SEND_BACKS: u32 = 3;

/// A gate the harness runs by itself, for a lead that was done without one at the tip of its branch.
#[derive(Debug)]
pub(super) enum OwnGate {
    /// It waits or runs.
    Running {
        /// The tip it was started for.
        tip: CommitId,
        /// Whether it is run once more after a gate had not passed at that tip.
        retry: bool,
        /// Where the gate service answers when it is over.
        answer: oneshot::Receiver<Result<Reply, Failure>>,
    },
    /// It ran: the record says what it found.
    Ran {
        /// The tip it was started for.
        tip: CommitId,
        /// Whether it was run once more after a gate had not passed at that tip.
        retry: bool,
    },
}

/// Where the harness's own gate for the tip of a lead's branch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Own {
    /// None ran for this tip, or the one that was started could not run.
    NotRun,
    /// One waits or runs; it may be for an earlier tip.
    Running,
    /// One ran for this tip.
    Ran {
        /// Whether it was run once more after a gate had not passed at the tip.
        retry: bool,
    },
}

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

/// The tip of the branch of an item, and whether the lead's worktree has that tip out.
pub(super) struct ItemTip {
    /// The commit the branch of the item points at.
    pub(super) tip: CommitId,
    /// Whether the worktree's `HEAD` is that commit: only then does a gate that runs in the worktree see the item's work.
    pub(super) checked_out: bool,
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
    /// The lead is done and the gate has not passed at `tip`: it is sent back.
    SentBack {
        /// The tip of its branch.
        tip: CommitId,
        /// Why the gate does not count.
        hold: GateHold,
    },
    /// A fix pass claimed a fix but handed back the reviewed commit.
    Uncommitted {
        /// The unchanged reviewed commit.
        tip: CommitId,
        /// Catalog-rendered reason for the human notice.
        reason: String,
    },
    /// The lead is done, but the gate has not passed at `tip` of the item's branch and its worktree has another branch out,
    /// where the gate of the harness would not see `tip`: nothing is pushed, and the lead is sent back to the branch.
    OffBranch {
        /// The tip of the item's branch.
        tip: CommitId,
        /// The item's branch.
        branch: BranchName,
        /// What the worktree has out, as the lead is told it.
        checked_out: String,
    },
    /// The lead is done and the gate passed, but no finished branch was joined.
    NotJoined {
        /// The tip of its branch.
        tip: CommitId,
        /// The finished items, none of whose branches the tip contains.
        missing: Vec<Joining>,
    },
}

/// Whether the text a lead is sent back with says how many times it was sent back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Count {
    /// The text names `sent` and `limit`.
    Told,
    /// The text names neither.
    Silent,
}

/// What a lead that is done is sent back to its branch with.
struct Back {
    /// Whether its text names how often the lead was sent back.
    count: Count,
    /// The text it is given.
    prompt: AutopilotPrompt,
    /// The variable of that text that says what is wrong, beside the branch and the count: its name and its value.
    variable: (&'static str, String),
    /// The outcome the log names.
    outcome: &'static str,
    /// What is wrong, as the human is told it when the lead was sent back in vain: a sentence.
    told: String,
    /// What the lead ended without, each time, for the human: the end of "ended again without".
    undone: &'static str,
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

/// Makes `origin` carry `ready.tip` on the branch decided by [`agent_core::decide::publish::publish_decision`]: pushes it, unless `origin` has it
/// there already (`carried`, what GitHub says the branch is at).
async fn carry(
    ready: &Ready,
    round: agent_core::decide::publish::RoundPublication,
    branches: &agent_core::decide::publish::PublicationBranches<'_>,
    carried: Option<&CommitId>,
) -> Result<BranchName, PublishError> {
    let remote_state = match carried {
        Some(head) if head == &ready.tip => agent_core::decide::publish::RemoteState::Equal,
        Some(head) => {
            if ready
                .git
                .is_ancestor(&Revision::Commit(head.clone()), &Revision::Commit(ready.tip.clone()))
                .await?
            {
                agent_core::decide::publish::RemoteState::Behind
            } else {
                agent_core::decide::publish::RemoteState::Diverged
            }
        }
        None => agent_core::decide::publish::RemoteState::Behind,
    };
    let decided = agent_core::decide::publish::publish_decision(round, branches, remote_state);
    match decided.action {
        agent_core::decide::publish::PublishAction::UpToDate => {
            ready.git.adopt_pushed(decided.branch, &ready.tip).await?;
        }
        agent_core::decide::publish::PublishAction::Push => {
            ready.git.push_commit(decided.branch, &ready.tip).await?;
        }
        agent_core::decide::publish::PublishAction::Refused => {
            return Err(PublishError::Refused {
                branch: decided.branch.clone(),
            });
        }
    }
    ready.git.branch_point_if_unheld(decided.branch, &ready.tip).await?;
    Ok(decided.branch.clone())
}

/// The item IDs `publication` is for.
fn item_ids_of(publication: &Publication) -> Vec<ItemId> {
    match &publication.publish {
        Publish::Open { items } => items.iter().map(|j| j.item).collect(),
        Publish::Update { fixing, joining, .. } => {
            fixing.iter().copied().chain(joining.iter().map(|j| j.item)).collect()
        }
    }
}

/// The numbers of the items `publication` is for.
fn items_of(publication: &Publication) -> Vec<String> {
    match &publication.publish {
        Publish::Open { items } => items.iter().map(|joining| joining.item.to_string()).collect(),
        Publish::Update { fixing, joining, .. } => fixing
            .iter()
            .copied()
            .chain(joining.iter().map(|joining| joining.item))
            .map(|id| id.to_string())
            .collect(),
    }
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

/// Decides and carries the publication branch for an open pull request.
async fn carry_open_pull(
    ready: &Ready,
    head_branch: &BranchName,
    ids: &[ItemId],
    team: Option<&agent_core::state::Team>,
    existing: Option<&RemotePullRequest>,
) -> Result<BranchName, PublishError> {
    let pr_branch = existing.as_ref().map(|found| &found.head_branch);
    let part_branch = if ids.iter().any(|id| {
        team.is_some_and(|team| {
            returned_delivery(team, *id) || team.round_of(*id).is_some_and(|round| round.part().is_some())
        })
    }) {
        Some(head_branch)
    } else {
        None
    };
    let branches = agent_core::decide::publish::PublicationBranches {
        task: head_branch,
        part: part_branch,
        pr: pr_branch,
    };
    let round_pub = if part_branch.is_some() {
        agent_core::decide::publish::RoundPublication::Part
    } else {
        agent_core::decide::publish::RoundPublication::Item
    };
    carry(ready, round_pub, &branches, existing.map(|found| &found.head)).await
}

/// Why a publication failure question cannot be prepared or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PublishQuestionError {
    /// Preparing prompt variables failed.
    #[error("cannot prepare publication variables for task {task}")]
    Variables {
        /// The task.
        task: TaskId,
        /// What failed.
        #[source]
        source: agent_core::prompt::VariablesError,
    },
    /// Rendering the question prompt failed.
    #[error("cannot render publication question for task {task}")]
    Render {
        /// The task.
        task: TaskId,
        /// What failed.
        #[source]
        source: agent_runtime::autopilot_prompts::RenderAutopilotTextError,
    },
    /// Writing the question file failed.
    #[error("cannot write publication question file {path:?} for task {task}")]
    Write {
        /// The task.
        task: TaskId,
        /// The file path.
        path: std::path::PathBuf,
        /// What failed.
        #[source]
        source: agent_runtime::text_file::ReplaceError,
    },
    /// Reading the question file back failed.
    #[error("cannot read publication question file {path:?} for task {task}")]
    Read {
        /// The task.
        task: TaskId,
        /// The file path.
        path: std::path::PathBuf,
        /// What failed.
        #[source]
        source: crate::daemon::questions::QuestionsProblem,
    },
    /// The question could not be recorded in state.
    #[error("cannot record publication question for task {task}")]
    State {
        /// The task.
        task: TaskId,
        /// What failed.
        #[source]
        source: agent_core::state::QuestionError,
    },
}

/// The branch of the item in `phase`, when it has one.
fn phase_branch(phase: &ItemPhase) -> Option<&BranchName> {
    match phase {
        ItemPhase::Pending => None,
        ItemPhase::InProgress { branch, .. } | ItemPhase::Fixing { branch, .. } => Some(branch),
        ItemPhase::InReview { pr, .. } | ItemPhase::Approved { pr, .. } => Some(&pr.branch),
        ItemPhase::Done { completion } => match completion {
            Completion::Listed => None,
            Completion::Merged { pr, .. } => Some(&pr.branch),
        },
    }
}

impl Owner {
    /// The tip of `branch`, the branch of an item in the worktree of `lead`, and whether the worktree has it out.
    ///
    /// The tip is the tip of `branch` when the worktree has the work of another item of the team out — its branch, or a
    /// detached `HEAD` at that branch's tip: what is committed there is not the work of this item. Otherwise the work is what
    /// the worktree has out — also a detached `HEAD` or a branch of the lead's own that goes on from the item — and its `HEAD`
    /// is the tip.
    ///
    /// # Errors
    /// [`PublishError`] when the worktree's git cannot be opened or cannot name a commit, or when another item is out and
    /// neither `branch` nor its last known tip on `origin` exists.
    pub(super) async fn item_tip(&self, lead: &Lead, branch: &BranchName) -> Result<ItemTip, PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let head = git.rev_parse(&Revision::Head).await?;
        let tip = git.branch_tip(branch).await?;
        if tip.as_ref() == Some(&head) {
            return Ok(ItemTip {
                tip: head,
                checked_out: true,
            });
        }
        if !self.another_item_is_out(&git, lead, branch, &head).await? {
            // The work that the worktree has out is the item's: the item's branch is out, or the branch is no item's — a part's,
            // a returned delivery's or the lead's own — and is made of what the worktree has out.
            return Ok(ItemTip {
                tip: head,
                checked_out: true,
            });
        }
        // Another item's work is out: what is committed there is not this item's, so the item's own tip is the one to judge
        // and push. Without a local branch that is the last tip known on `origin`; when that is unknown too, there is no tip
        // to name, and the error says so instead of publishing what the worktree has out.
        let tip = match tip {
            Some(tip) => tip,
            None => git.rev_parse(&Revision::RemoteBranch(branch.clone())).await?,
        };
        Ok(ItemTip {
            tip,
            checked_out: false,
        })
    }

    /// Whether the worktree of `lead`, which has `head` out, has the work of an item of its team other than the one on
    /// `branch`: the branch of such an item is out, or `HEAD` is detached at the tip of one.
    async fn another_item_is_out(
        &self,
        git: &Git,
        lead: &Lead,
        branch: &BranchName,
        head: &CommitId,
    ) -> Result<bool, PublishError> {
        let items = self.item_branches(lead);
        // A branch that is no item's — a part's, a returned delivery's, or the lead's own, which integrates the work of the
        // items — is made of what the worktree has out, whichever item that is.
        if !items.contains(branch) {
            return Ok(false);
        }
        if let Some(out) = git.current_branch().await? {
            return Ok(out != *branch && items.contains(&out));
        }
        for other in items.into_iter().filter(|other| other != branch) {
            if git.branch_tip(&other).await?.as_ref() == Some(head) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The branches of the items of the team of `lead`, in any phase.
    fn item_branches(&self, lead: &Lead) -> Vec<BranchName> {
        self.parts.state.find_task(&lead.task).map_or_else(Vec::new, |found| {
            found
                .team
                .items()
                .iter()
                .filter_map(|item| phase_branch(item.phase()).cloned())
                .collect()
        })
    }

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
            if let Publish::Update { fixing, .. } = &publication.publish
                && self.held_for_response(team_id, fixing)
            {
                continue;
            }

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
            if self.recover_publish_question(site, &publication, now).await? {
                continue;
            }
            let items = items_of(&publication);
            let rounds = self.publication_rounds(team_id, &items);
            for round in &rounds {
                self.watch.waits.remove(round);
            }
            let attempt = self.try_publish(site, &publication, now).await;
            self.remember_publication_wait(&rounds, &attempt);
            match attempt {
                Ok(Attempt::Nothing) => {}
                Ok(Attempt::Published(published)) => {
                    self.watch.retry_at.remove(&key);
                    self.watch.gate_sent_back.remove(&lead.task);
                    self.watch.gate_held.remove(&lead.task);
                    self.watch.own_gates.remove(&lead.task);
                    self.each_item(team_id, &published.items, |item| {
                        item.reset_publish_tries();
                        item.clear_publish_question();
                        Ok(())
                    })?;
                    self.save().await?;
                    if self.record_publication(team_id, &publication, &published, look)? {
                        self.note_reviewed(&lead, &published.pr.branch, &published.gated).await;
                    }
                }
                Ok(Attempt::Held(held)) => {
                    self.watch.retry_at.remove(&key);
                    self.hold_round(pulls, team_id, &lead, held, look)?;
                }
                Ok(Attempt::SentBack { tip, hold }) => {
                    self.send_back_for_gate(site, &publication, (&tip, &hold), now, look)
                        .await?;
                }
                Ok(Attempt::Uncommitted { tip, reason }) => {
                    let back = Back {
                        count: Count::Told,
                        prompt: AutopilotPrompt::ReviewFixNoCommit,
                        variable: ("tip", tip.to_string()),
                        outcome: "fix_without_commit",
                        told: reason,
                        undone: include_str!("../../../../../../prompts/harness/review/fix-no-commit-undone.md"),
                    };
                    self.send_lead_back(site, &publication, &tip, &back, now, look).await?;
                }
                Ok(Attempt::OffBranch {
                    tip,
                    branch,
                    checked_out,
                }) => {
                    self.send_back_to_branch(site, &publication, (&tip, &branch, &checked_out), now, look)
                        .await?;
                }
                Ok(Attempt::NotJoined { tip, missing }) => {
                    self.send_back_to_join(site, &publication, (&tip, &missing), now, look)
                        .await?;
                }
                Err(error) => self.publish_failed(site, &publication, &error, now, look).await?,
            }
        }
        Ok(())
    }

    /// Finds the rounds whose items this publication carries.
    fn publication_rounds(&self, team: &TeamId, items: &[String]) -> Vec<agent_core::ids::RoundId> {
        self.parts
            .state
            .team(team)
            .into_iter()
            .flat_map(agent_core::state::Team::rounds)
            .filter(|round| round.item().is_some_and(|id| items.contains(&id.to_string())))
            .map(|round| round.id().clone())
            .collect()
    }

    /// Retains the pending operation when publication leaves an idle round open.
    fn remember_publication_wait(
        &mut self,
        rounds: &[agent_core::ids::RoundId],
        attempt: &Result<Attempt, PublishError>,
    ) {
        let obligation = match attempt {
            Ok(Attempt::Nothing | Attempt::Published(_)) => None,
            Ok(Attempt::Held(super::checks::Held::Waiting | super::checks::Held::Notify)) => Some(Obligation::Human),
            Ok(
                Attempt::Held(super::checks::Held::WakeLead)
                | Attempt::SentBack { .. }
                | Attempt::OffBranch { .. }
                | Attempt::Uncommitted { .. }
                | Attempt::NotJoined { .. },
            ) => Some(Obligation::LeadCorrection),
            Err(_) => Some(Obligation::Review),
        };
        if let Some(obligation) = obligation {
            for round in rounds {
                self.watch.waits.insert(round.clone(), obligation);
            }
        }
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
        let ItemTip { tip, checked_out } = self.item_tip(lead, &publication.head_branch).await?;
        // A gate of the harness that still runs decides nothing yet; the record is read after it, so that it is not older.
        let own = self.own_gate_state(&lead.task, &tip);
        if own == Own::Running {
            return Ok(Attempt::Nothing);
        }
        let record = self.parts.store.load_gate_record(&lead.task).await?;
        let base = Revision::Branch(self.base_branch(&lead.repo)?);
        let waiting = Waiting {
            tip: &tip,
            lead_done: lead.done,
            // A tip the base branch has is the base branch: the lead committed nothing.
            own_work: !git.is_ancestor(&Revision::Commit(tip.clone()), &base).await?,
            record: record.as_ref(),
            harness_gate: match own {
                Own::NotRun | Own::Running => HarnessGate::NotRun,
                Own::Ran { retry: false } => HarnessGate::Ran,
                Own::Ran { retry: true } => HarnessGate::Retried,
            },
        };
        let record = match (gate_step(&waiting), record) {
            (GateStep::Publish, Some(record)) => record,
            (GateStep::Publish | GateStep::Wait, _) => return Ok(Attempt::Nothing),
            // The gate of the harness runs in the worktree: it sees the tip of the item's branch only when the worktree has it out.
            (GateStep::Run | GateStep::Retry, _) if !checked_out => {
                let checked_out = super::super::item_branch::named(git.current_branch().await?.as_ref());
                return Ok(Attempt::OffBranch {
                    tip,
                    branch: publication.head_branch.clone(),
                    checked_out,
                });
            }
            (GateStep::Run, _) => {
                self.start_own_gate(&lead.task, &tip, false);
                return Ok(Attempt::Nothing);
            }
            (GateStep::Retry, _) => {
                self.start_own_gate(&lead.task, &tip, true);
                return Ok(Attempt::Nothing);
            }
            (GateStep::SendBack(hold), _) => return Ok(Attempt::SentBack { tip, hold }),
        };
        let ready = Ready { git, tip, record };
        // The checkpoints of the steps so far are in the history before the branch is pushed.
        self.flush_checkpoints().await;
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

    /// Finds the open pull request or creates a draft pull request for the publication.
    async fn find_or_create_pull_request(
        &mut self,
        site: Site<'_>,
        head_branch: &BranchName,
        ids: &[ItemId],
        ready: &Ready,
        existing: Option<RemotePullRequest>,
    ) -> Result<RemotePullRequest, PublishError> {
        let Site { pulls, team, lead } = site;
        let text = self.pull_text_of(pulls, team, &lead.repo, ids, &ready.record).await?;
        let base = self.base_branch(&lead.repo)?;
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        if let Some(mut found) = existing {
            if found.head != ready.tip {
                found.head = ready.tip.clone();
                self.record_event(
                    team,
                    self.rounds_of_items(team, ids),
                    HistoryEvent::PullPushed {
                        pr: found.number,
                        head: ready.tip.clone(),
                    },
                );
            }
            Ok(found)
        } else {
            self.flush_checkpoints().await;
            let body = self.link_commits(&ready.git, &ready.tip, &text.body).await;
            let created = github
                .create_pull_request(&NewPullRequest {
                    title: text.title,
                    body,
                    head: head_branch.clone(),
                    base,
                    readiness: Readiness::Draft,
                })
                .await?;
            self.record_event(
                team,
                self.rounds_of_items(team, ids),
                HistoryEvent::PullOpened {
                    number: created.number,
                    branch: head_branch.clone(),
                    head: created.head.clone(),
                    title: created.title.clone(),
                    body: created.body.clone(),
                },
            );
            Ok(created)
        }
    }

    /// Finalizes description and marks draft PR ready for review.
    async fn finalize_opened_pull(
        &mut self,
        site: Site<'_>,
        ids: &[ItemId],
        ready: &Ready,
        remote: &RemotePullRequest,
    ) -> Result<(), PublishError> {
        let Site { pulls, team, lead } = site;
        let text = self.pull_text_of(pulls, team, &lead.repo, ids, &ready.record).await?;
        let body = self.disclosed(lead, ready, &text.body).await?;
        let body = if body == remote.body {
            body
        } else {
            refresh_harness_files(&ready.git, &body, &text.body).await
        };
        let body = self.link_commits(&ready.git, &ready.tip, &body).await;
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        if body != remote.body {
            let update = PullRequestUpdate {
                body: Some(body),
                ..PullRequestUpdate::default()
            };
            github.update_pull_request(remote.number, &update).await?;
        }
        // The checkpoint of the opening is in the history before the pull request is marked ready.
        self.flush_checkpoints().await;
        github.mark_ready(remote.number).await?;
        self.record_event(
            team,
            self.rounds_of_items(team, ids),
            HistoryEvent::PullReady { pr: remote.number },
        );
        Ok(())
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
            // A lead that is done handed its branch over: with none of the finished items in it nothing would ever be
            // published, and nobody would be told. A lead that is not done is still at it.
            return Ok(if lead.done && !items.is_empty() {
                Attempt::NotJoined {
                    tip: ready.tip.clone(),
                    missing: items.to_vec(),
                }
            } else {
                Attempt::Nothing
            });
        }
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        let existing = github.find_open_pull_request(head_branch).await?;
        let published_branch =
            carry_open_pull(ready, head_branch, &ids, self.parts.state.team(team), existing.as_ref()).await?;
        let head_branch = &published_branch;
        let remote = self
            .find_or_create_pull_request(site, head_branch, &ids, ready, existing)
            .await?;
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
        self.finalize_opened_pull(site, &ids, ready, &remote).await?;
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

    /// Whether the update of a pull request whose items `fixing` are fixed waits: the lead's fix response to the pull
    /// request's fix pass is not taken yet, or a second opinion of that pass is still out or confirmed a blocker.
    fn held_for_response(&self, team_id: &TeamId, fixing: &[ItemId]) -> bool {
        let Some(team) = self.parts.state.team(team_id) else {
            return false;
        };
        fixing.iter().filter_map(|id| team.item(*id)).any(|item| {
            let passes = crate::daemon::merge::pull_passes(team, item);
            item.findings().is_some_and(|ledger| {
                ledger.disputes().is_some_and(|d| d.answered_pass != passes)
                    || std::num::NonZeroU32::new(passes).is_some_and(|pass| {
                        matches!(
                            agent_core::decide::opinion::after_opinions(
                                ledger,
                                pass,
                                &ledger.fixed_answers(pass),
                                agent_core::decide::opinion::Tip::Changed
                            ),
                            agent_core::decide::opinion::Next::AwaitOpinions
                                | agent_core::decide::opinion::Next::FixConfirmed(_)
                        )
                    })
            })
        })
    }

    /// Checks whether every blocking finding was answered without claiming a committed fix.
    fn unchanged_answers(&self, team: &TeamId, fixing: &[ItemId]) -> Answers {
        let accounted = fixing.iter().all(|id| {
            self.parts
                .state
                .team(team)
                .and_then(|t| Some((t, t.item(*id)?)))
                .is_some_and(|(team, item)| {
                    item.findings().is_some_and(|ledger| {
                        NonZeroU32::new(crate::daemon::merge::pull_passes(team, item))
                            .is_none_or(|pass| ledger.fixed_answers(pass).is_empty())
                            && ledger
                                .entries()
                                .iter()
                                .filter(|entry| {
                                    entry.status == agent_core::state::FindingStatus::Open
                                        && entry.finding.severity.blocks()
                                })
                                .all(|entry| {
                                    ledger.disputes().is_some_and(|disputes| {
                                        disputes.explanations.get(&entry.number).is_some_and(|explanation| {
                                            explanation.status == agent_core::state::ExplanationStatus::Pending
                                        })
                                    })
                                })
                    })
                })
        });
        if accounted {
            Answers::Accounted
        } else {
            Answers::Missing
        }
    }

    /// Member branches that must be integrated before the existing pull request can be updated.
    fn member_fix_branches(&self, team: &TeamId, fixing: &[ItemId], lead: &Lead) -> Vec<Joining> {
        self.parts
            .state
            .team(team)
            .into_iter()
            .flat_map(|team| fixing.iter().filter_map(|id| team.item(*id)))
            .filter_map(|item| match item.phase() {
                ItemPhase::Fixing { assignee, branch, .. } if assignee != &lead.agent => Some(Joining {
                    item: item.id(),
                    branch: branch.clone(),
                }),
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::InReview { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Approved { .. }
                | ItemPhase::Done { .. } => None,
            })
            .collect()
    }

    /// Updates the pull request body and comments for reviewers when a fix pass is published.
    async fn update_fix_details(
        &mut self,
        site: Site<'_>,
        pr: &PullRequest,
        fixing: &[ItemId],
        ids: &[ItemId],
        ready: &Ready,
        before: &RemotePullRequest,
    ) -> Result<RemotePullRequest, PublishError> {
        let Site { pulls, team, lead } = site;
        let text = self.pull_text_of(pulls, team, &lead.repo, ids, &ready.record).await?;
        let mut update = PullRequestUpdate::default();
        if text.title != before.title {
            update.title = Some(text.title);
        }
        let body = self.disclosed(lead, ready, &text.body).await?;
        let body = refresh_harness_files(&ready.git, &body, &text.body).await;
        let body = self.link_commits(&ready.git, &ready.tip, &body).await;
        if body != before.body {
            update.body = Some(body);
        }
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        if update != PullRequestUpdate::default() {
            github.update_pull_request(pr.number, &update).await?;
        }
        let comment = pulls.texts.render(
            AutopilotPrompt::PrUpdate,
            &Variables::new()
                .with("head", ready.tip.as_str())?
                .with("pass", self.fix_passes_of(team, fixing).to_string())?
                .with("gate", ready.record.summary())?,
        )?;
        let comment = self.link_commits(&ready.git, &ready.tip, &comment).await;
        github.post_comment(pr.number, comment.trim_end()).await?;
        Ok(github.get_pull_request(pr.number).await?)
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
        let member_fixes = self.member_fix_branches(team, fixing, lead);
        if contained(&ready.git, &member_fixes, &ready.tip).await?.len() != member_fixes.len() {
            return Ok(Attempt::Nothing);
        }
        let joined = contained(&ready.git, joining, &ready.tip).await?;
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        let before = github.get_pull_request(pr.number).await?;
        // What the reviewers saw is what the harness put in review, not what GitHub carries: a fix may be on `origin`
        // before the harness looks (the agent pushed it, or GitHub showed the harness's own push late). A branch that
        // went in review before the harness kept this is judged by what GitHub carries.
        let reviewed = ready
            .git
            .reviewed_tip(&pr.branch)
            .await?
            .unwrap_or_else(|| before.head.clone());
        if reviewed == ready.tip {
            if !lead.done {
                return Ok(Attempt::Nothing);
            }
            let answers = self.unchanged_answers(team, fixing);
            let budget = if self.watch.gate_sent_back.get(&lead.task).copied().unwrap_or(0) < GATE_SEND_BACKS {
                Budget::Available
            } else {
                Budget::Exhausted
            };
            match fix_step(Progress::Unchanged(answers), budget) {
                Step::Correct(_) | Step::AskHuman(_) => {
                    return Ok(Attempt::Uncommitted {
                        tip: ready.tip.clone(),
                        reason: pulls.texts.render(
                            AutopilotPrompt::ReviewFixNoCommitReason,
                            &Variables::new().with("tip", ready.tip.as_str())?,
                        )?,
                    });
                }
                Step::Verify => {}
                Step::WaitGate => return Ok(Attempt::Nothing),
            }
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
        let branches = agent_core::decide::publish::PublicationBranches {
            task: &pr.branch,
            part: None,
            pr: Some(&pr.branch),
        };
        carry(
            ready,
            agent_core::decide::publish::RoundPublication::FixPass,
            &branches,
            Some(&before.head),
        )
        .await?;
        self.record_event(
            team,
            self.rounds_of_items(team, &ids),
            HistoryEvent::PullPushed {
                pr: pr.number,
                head: ready.tip.clone(),
            },
        );
        let after = self.update_fix_details(site, pr, fixing, &ids, ready, &before).await?;
        Ok(Attempt::Published(Published {
            pr: pr.clone(),
            gated: ready.tip.clone(),
            carried: after.head,
            items: ids,
        }))
    }

    /// Refreshes descriptions independently of publication, including file-only changes during review.
    pub(super) async fn refresh_descriptions(&mut self, pulls: &Pulls, team: &TeamId) -> Result<(), PublishError> {
        let found = self.parts.state.team(team).ok_or(PublishError::Lost)?;
        let mut open = std::collections::BTreeMap::new();
        for item in found.items() {
            let pr = match item.phase() {
                ItemPhase::InReview { pr, .. } | ItemPhase::Approved { pr, .. } | ItemPhase::Fixing { pr, .. } => pr,
                ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => continue,
            };
            open.entry(pr.number).or_insert_with(Vec::new).push(item.id());
        }
        if open.is_empty() {
            return Ok(());
        }
        let (_, member) = found.crew().roster().into_iter().next().ok_or(PublishError::Lost)?;
        let lead = Lead {
            repo: found.repo().clone(),
            task: member.task().clone(),
            agent: member.agent().name().parse()?,
            done: false,
        };
        let Some(record) = self.parts.store.load_gate_record(&lead.task).await? else {
            return Ok(());
        };
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        for (number, ids) in open {
            let remote = github.get_pull_request(number).await?;
            if remote.state != agent_runtime::github::PullState::Open || remote.head != record.result.head {
                continue;
            }
            let ready = Ready {
                git: git.clone(),
                tip: remote.head.clone(),
                record: record.clone(),
            };
            let text = self.pull_text_of(pulls, team, &lead.repo, &ids, &record).await?;
            let body = self.disclosed(&lead, &ready, &text.body).await?;
            let body = refresh_harness_files(&git, &body, &text.body).await;
            let body = self.link_commits(&git, &remote.head, &body).await;
            if body != remote.body {
                github
                    .update_pull_request(
                        number,
                        &PullRequestUpdate {
                            body: Some(body),
                            ..PullRequestUpdate::default()
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// Renders only the harness header, so authored file disclosures remain distinguishable.
    pub(super) fn harness_text_of(
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

    /// The title and the description of the pull request of `ids`.
    pub(super) async fn pull_text_of(
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
        let (_, lead) = team.crew().roster().into_iter().next().ok_or(PublishError::Lost)?;
        let git = self.worktree_git(repo, lead.task(), &lead.agent().name().parse()?)?;
        let path = self
            .parts
            .store
            .home()
            .task_dir(lead.task())
            .join(crate::daemon::pr::description::LEAD_DESCRIPTION_FILE);
        let contribution =
            agent_runtime::text_file::read_text_file(&path, crate::daemon::pr::description::MAX_DESCRIPTION_BYTES)
                .await
                .map_err(|source| PublishError::Description { path, source })?;
        let mut contribution =
            crate::daemon::pr::description::lead_sections(contribution.as_ref().map_or("", |file| file.text.as_str()));
        if contribution.is_empty() {
            contribution = pulls.texts.render(AutopilotPrompt::PrNoLead, &Variables::new())?;
        }
        let base = Revision::RemoteBranch(self.base_branch(repo)?);
        let size = git.pull_request_size(&base, &record.result.head).await?;
        let previous = self.parts.store.previous_gate_record(lead.task()).await?;
        let measured = crate::daemon::pr::description::measured_sections_in(
            &pulls.texts,
            size,
            record,
            previous.as_ref(),
            self.parts.zone,
        )?;
        let mut text = self.harness_text_of(pulls, team_id, repo, ids, record)?;
        let allowed = if team.allowances().is_empty() {
            None
        } else {
            let merge_base = git
                .merge_base(&base, &Revision::Commit(record.result.head.clone()))
                .await?;
            let files = git.range_changes(&merge_base, &record.result.head).await?;
            crate::daemon::pr::description::allowed_section(&pulls.texts, &files, team.allowances())?
        };
        let mut crew = Vec::new();
        for (_, member) in team.crew().roster() {
            crew.push((member.task().clone(), member.agent().name().parse::<AgentName>()?));
        }
        let (requests, with_summaries) = self.requests_of_crew(&crew).await?;
        let worker_steps = crate::daemon::pr::description::worker_steps_section(&pulls.texts, &requests)?;
        let returned_steps = crate::daemon::pr::description::returned_steps_section(&pulls.texts, &with_summaries)?;
        text.body = [text.body.trim_end(), contribution.trim(), &measured].join("\n\n");
        for section in [allowed, worker_steps, returned_steps].into_iter().flatten() {
            text.body.push_str("\n\n");
            text.body.push_str(section.trim_end());
        }
        Ok(text)
    }

    /// The requests for workers of every task of a crew, as the bus lists them, and again with the summary of each
    /// request that came back to its requester and names a step (empty for the others).
    async fn requests_of_crew(
        &self,
        crew: &[(TaskId, AgentName)],
    ) -> Result<
        (
            Vec<(AgentName, Vec<TaskRequest>)>,
            Vec<(AgentName, Vec<RequestWithSummary>)>,
        ),
        PublishError,
    > {
        let mut requests = Vec::new();
        let mut with_summaries = Vec::new();
        for (task, requester) in crew {
            let bus_error = |source| PublishError::Bus {
                task: task.clone(),
                source,
            };
            let of_task = self.parts.bus.helper_dump(task).await.map_err(bus_error)?;
            let mut read = Vec::with_capacity(of_task.len());
            for request in &of_task {
                let summary = match (&request.status, request.step) {
                    (DelegationStatus::Failed, Some(_)) => {
                        self.parts.bus.helper_get(request.id).await.map_err(bus_error)?.summary
                    }
                    _ => String::new(),
                };
                read.push((request.clone(), summary));
            }
            with_summaries.push((requester.clone(), read));
            requests.push((requester.clone(), of_task));
        }
        Ok((requests, with_summaries))
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

    /// How many review passes `items` of `team` have completed before their current one: their fix passes and the merge
    /// conflict passes they have spent.
    pub(super) fn review_passes_of(&self, team: &TeamId, items: &[ItemId]) -> usize {
        items
            .iter()
            .filter_map(|id| self.parts.state.team(team).and_then(|found| found.item(*id)))
            .map(agent_core::state::ItemProgress::completed_review_passes)
            .max()
            .unwrap_or(0)
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
                    tracing::warn!(
                        event = LogEvent::GithubCallFailed.as_str(),
                        team = %team_id,
                        item = %id,
                        error = %error_chain(&error),
                        "a pull request does not carry what the gate passed: it is looked at again"
                    );
                }
            }
        }
        confirmed
    }

    /// Records that the items of `published` are in review, after the round machine checked that the pull request carries
    /// the commit the gate passed on. Returns whether any item went in review.
    pub(super) fn record_publication(
        &mut self,
        team_id: &TeamId,
        publication: &Publication,
        published: &Published,
        look: &mut Look,
    ) -> Result<bool, OwnerError> {
        let confirmed = self.confirmed(team_id, publication, published);
        // A joining item shares the PR ledger, including the legacy protocol and existing comment ID.
        let findings = match &publication.publish {
            Publish::Open { .. } => Some(agent_core::state::Findings::verifying(
                self.parts.settings.config.review().min_first_full_reviews(),
            )),
            Publish::Update { fixing, .. } => fixing
                .first()
                .and_then(|id| self.parts.state.team(team_id)?.item(*id)?.findings())
                .cloned(),
        };
        let in_review = !confirmed.is_empty();
        for id in confirmed {
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
            if opened && let Some(findings) = &findings {
                item.start_findings();
                if let Some(ledger) = item.findings_mut() {
                    ledger.clone_from(findings);
                }
            }
            let round = self.round_of_item(team_id, id)?;
            if opened {
                look.events.push(EventKind::PullRequestOpened {
                    team: team_id.clone(),
                    item: id,
                    round: round.clone(),
                    pr: published.pr.number,
                });
            }
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: id,
                round: round.clone(),
                status: agent_core::status::ItemStatus::InReview,
            });
            look.changed = true;
            let span = tracing::error_span!("round", team = %team_id, rounds = %round);
            let _entered = span.enter();
            tracing::info!(
                event = LogEvent::ItemPhaseChanged.as_str(),
                team = %team_id,
                item = %id,
                phase = "in_review",
                pr = %published.pr.number,
                "an item is in review"
            );
        }
        Ok(in_review)
    }

    /// Keeps `tip` as what the reviewers of `branch` were asked to look at. A tip that cannot be kept costs at most a fix
    /// that waits for the next commit of the lead, so the round goes on; git said why in the log.
    async fn note_reviewed(&self, lead: &Lead, branch: &BranchName, tip: &CommitId) {
        let kept = match self.worktree_git(&lead.repo, &lead.task, &lead.agent) {
            Ok(git) => git.record_reviewed(branch, tip).await.map_err(PublishError::from),
            Err(error) => Err(error),
        };
        if let Err(error) = kept {
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                branch = %branch,
                reason = %error_chain(&error),
                "what the reviewers were asked to look at cannot be kept"
            );
        }
    }

    /// Where the gate the harness started by itself for `task` is, for the branch tip `tip`. A gate that is over is taken
    /// off the service: one that ran is remembered with its tip, one that could not run is forgotten and started again.
    pub(super) fn own_gate_state(&mut self, task: &TaskId, tip: &CommitId) -> Own {
        let over = match self.watch.own_gates.get_mut(task) {
            Some(OwnGate::Running {
                tip: started,
                retry,
                answer,
            }) => match answer.try_recv() {
                Err(oneshot::error::TryRecvError::Empty) => return Own::Running,
                Ok(Ok(_)) => Some(Ok((started.clone(), *retry))),
                Ok(Err(failure)) => Some(Err(failure.message)),
                Err(oneshot::error::TryRecvError::Closed) => {
                    Some(Err("the gate service ended before it answered".to_owned()))
                }
            },
            Some(OwnGate::Ran { .. }) | None => None,
        };
        match over {
            Some(Ok((started, retry))) => {
                self.watch
                    .own_gates
                    .insert(task.clone(), OwnGate::Ran { tip: started, retry });
            }
            Some(Err(reason)) => {
                tracing::warn!(
                    event = LogEvent::GateCannotStart.as_str(),
                    task = %task,
                    %reason,
                    "the gate the harness runs for a lead that is done did not run: it is started again"
                );
                self.watch.own_gates.remove(task);
            }
            None => {}
        }
        match self.watch.own_gates.get(task) {
            Some(OwnGate::Ran { tip: ran, retry }) if ran == tip => Own::Ran { retry: *retry },
            Some(OwnGate::Ran { .. } | OwnGate::Running { .. }) | None => Own::NotRun,
        }
    }

    /// Starts the gate of `task` for the branch tip `tip`: nobody ran one, and the lead says it is done; or, with `retry`,
    /// one did not pass there and is run once more.
    pub(super) fn start_own_gate(&mut self, task: &TaskId, tip: &CommitId, retry: bool) {
        match self.own_gate(task) {
            Ok(answer) => {
                tracing::info!(
                    event = LogEvent::ChecksDecided.as_str(),
                    outcome = if retry { "gate_retried" } else { "gate_started" },
                    task = %task,
                    tip = %tip,
                    "the lead is done and no gate passed at the tip of its branch: the harness runs the gate"
                );
                self.watch.own_gates.insert(
                    task.clone(),
                    OwnGate::Running {
                        tip: tip.clone(),
                        retry,
                        answer,
                    },
                );
            }
            Err(failure) => {
                tracing::warn!(
                    event = LogEvent::GateCannotStart.as_str(),
                    task = %task,
                    reason = %failure.message,
                    "the gate the harness runs for a lead that is done cannot be started: it is tried again"
                );
            }
        }
    }

    /// The lead is done and the gate has not passed at the tip of its branch: it is sent back with the reason, and after
    /// [`GATE_SEND_BACKS`] times the human is told.
    async fn send_back_for_gate(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        (tip, hold): (&CommitId, &GateHold),
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let back = Back {
            count: Count::Told,
            prompt: AutopilotPrompt::GateNotPassed,
            variable: ("reason", hold.to_string()),
            outcome: "gate_sent_back",
            told: format!(
                "the gate has not passed at the tip of the branch {}: {hold}",
                publication.head_branch
            ),
            undone: "a gate that passes",
        };
        self.send_lead_back(site, publication, tip, &back, now, look).await
    }

    /// The lead is done and no gate has passed at the tip of the item's `branch`, while its worktree has `checked_out` out:
    /// the harness pushes nothing and the lead is sent back to switch to the branch and run the gate there, and after
    /// [`GATE_SEND_BACKS`] times the human is told.
    async fn send_back_to_branch(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        (tip, branch, checked_out): (&CommitId, &BranchName, &str),
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        tracing::warn!(
            event = LogEvent::PublishOffBranch.as_str(),
            team = %site.team,
            task = %site.lead.task,
            branch = %branch,
            checked_out,
            tip = %tip,
            "the branch of the item has no gate at its tip and the worktree has another branch out: nothing is pushed"
        );
        let back = Back {
            count: Count::Silent,
            prompt: AutopilotPrompt::ReviewOffBranch,
            variable: ("checked_out", checked_out.to_owned()),
            outcome: "off_branch",
            told: format!(
                "the worktree has {checked_out} out, and no gate has passed at the tip of the branch {branch}."
            ),
            undone: "its worktree on the branch and a gate that passes there",
        };
        self.send_lead_back(site, publication, tip, &back, now, look).await
    }

    /// The lead is done, the gate passed, and its branch contains the branch of none of the finished items in `missing`:
    /// it is sent back to merge them, and after [`GATE_SEND_BACKS`] times the human is told.
    async fn send_back_to_join(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        (tip, missing): (&CommitId, &[Joining]),
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lines: Vec<String> = missing
            .iter()
            .map(|joining| format!("- item {}: `{}`", joining.item, joining.branch))
            .collect();
        let named: Vec<String> = missing
            .iter()
            .map(|joining| format!("item {}: {}", joining.item, joining.branch))
            .collect();
        let back = Back {
            count: Count::Told,
            prompt: AutopilotPrompt::BranchNotJoined,
            variable: ("missing", lines.join("\n")),
            outcome: "not_joined_sent_back",
            told: format!(
                "the gate passed, but the branch {} contains the branch of none of the finished items ({}).",
                publication.head_branch,
                named.join("; ")
            ),
            undone: "their branches merged",
        };
        self.send_lead_back(site, publication, tip, &back, now, look).await
    }

    /// Sends the lead, which is done, back to its branch with the text of `back`; after [`GATE_SEND_BACKS`] times in a row
    /// the human is told once instead, and the lead waits for them.
    async fn send_lead_back(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        tip: &CommitId,
        back: &Back,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Site { pulls, team, lead } = site;
        let sent = self.watch.gate_sent_back.get(&lead.task).copied().unwrap_or(0);
        if sent >= GATE_SEND_BACKS {
            self.watch.gate_held.insert(lead.task.clone());
            let announcement = Announcement::GateNotPassed {
                team: team.clone(),
                branch: publication.head_branch.clone(),
                tip: tip.clone(),
            };
            if self.watch.announced.insert(announcement) {
                let reason = format!(
                    "{} The lead was sent back {sent} times and ended again without {}; it waits for you.",
                    back.told, back.undone
                );
                let values = Variables::new()
                    .with("team", team.as_str())
                    .and_then(|values| values.with("items", items_of(publication).join(", ")))
                    .and_then(|values| values.with("reason", &reason));
                let items = items_of(publication);
                let rounds = self
                    .parts
                    .state
                    .team(team)
                    .into_iter()
                    .flat_map(agent_core::state::Team::rounds)
                    .filter(|round| round.item().is_some_and(|item| items.contains(&item.to_string())))
                    .map(|round| round.id().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let values = values
                    .and_then(|v| v.with("rounds", rounds))
                    .and_then(|v| v.with("task", lead.task.as_str()));
                self.tell_human(pulls, AutopilotNotice::HandoverHeld, values, lead, now);
                self.queue_for_task(&lead.task, CheckpointCause::WaitsForHuman);
            }
            return Ok(());
        }
        let sent = sent.saturating_add(1);
        let text = Variables::new()
            .with("branch", publication.head_branch.as_str())
            .and_then(|values| values.with(back.variable.0, back.variable.1.as_str()))
            .and_then(|values| match back.count {
                Count::Told => values
                    .with("sent", sent.to_string())
                    .and_then(|values| values.with("limit", GATE_SEND_BACKS.to_string())),
                Count::Silent => Ok(values),
            })
            .map_err(PublishError::from)
            .and_then(|values| Ok(pulls.texts.render(back.prompt, &values)?));
        let text = match text {
            Ok(text) => text,
            Err(error) => return self.publish_failed(site, publication, &error, now, look).await,
        };
        tracing::info!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = back.outcome,
            team = %team,
            task = %lead.task,
            tip = %tip,
            sent,
            reason = %back.told,
            "the lead is done and its branch cannot be published as it is: it is sent back"
        );
        self.watch.gate_sent_back.insert(lead.task.clone(), sent);
        self.release(&lead.task, Why::Gate(text.trim_end().to_owned()), look)
    }

    /// What follows a branch that could not be published: it is logged, tried again after a while, and the human is told
    /// once.
    pub(super) async fn publish_failed(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        error: &PublishError,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        if let PublishError::RepoGone { repo } = error {
            tracing::error!(
                event = LogEvent::ConfigurationNameMissing.as_str(),
                team = %site.team,
                kind = "repo",
                name = %repo,
                "a repository in the state is gone from the configuration"
            );
        }
        let reason = error_chain(error);
        let item_ids = item_ids_of(publication);
        if self
            .parts
            .state
            .find_task(&site.lead.task)
            .is_some_and(|found| found.member.open_question().is_some())
        {
            return Ok(());
        }
        let mut max_tries = 0;
        self.each_item(site.team, &item_ids, |item| {
            let tries = item.record_publish_try();
            if tries > max_tries {
                max_tries = tries;
            }
            Ok(())
        })?;
        let limit = self.parts.settings.config.publish().tries;
        let step = agent_core::decide::publish::publish_tries_step(max_tries, limit);
        log_decision(
            "publish_tries_step",
            &format!("tries={max_tries}, limit={limit}"),
            &format!("{step:?}"),
        );
        match step {
            agent_core::decide::publish::PublishTriesStep::Retry => {
                self.retry_publish(site, publication, &reason, (max_tries, limit), now)?;
                self.save().await?;
            }
            agent_core::decide::publish::PublishTriesStep::TellLead => {
                self.tell_lead_publish_failed(site, publication, &reason, (max_tries, limit), look)?;
                self.save().await?;
            }
            agent_core::decide::publish::PublishTriesStep::AskHuman => {
                self.ask_human_publish_failed(site, publication, &reason, (max_tries, limit), now)
                    .await?;
            }
        }
        Ok(())
    }

    /// Registers a question file left between atomic creation and the state commit.
    pub(super) async fn recover_publish_question(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        now: Timestamp,
    ) -> Result<bool, OwnerError> {
        let Some(team) = self.parts.state.team(site.team) else {
            return Ok(false);
        };
        let Some(found) = self.parts.state.find_task(&site.lead.task) else {
            return Ok(false);
        };
        if !matches!(found.member.status(), TaskStatus::Waiting | TaskStatus::Failed)
            || found.member.open_question().is_some()
        {
            return Ok(false);
        }
        let tries = item_ids_of(publication).iter().try_fold(0, |largest, id| {
            let item = team.item(*id).ok_or_else(|| OwnerError::LostItem {
                team: site.team.clone(),
                item: *id,
            })?;
            Ok::<_, OwnerError>(largest.max(item.publish_tries()))
        })?;
        let limit = self.parts.settings.config.publish().tries;
        if agent_core::decide::publish::publish_tries_step(tries, limit)
            != agent_core::decide::publish::PublishTriesStep::AskHuman
        {
            return Ok(false);
        }
        let number = super::super::start::next_number(found.member).ok_or_else(|| OwnerError::QuestionNumbers {
            task: site.lead.task.clone(),
        })?;
        let path = self.parts.store.home().questions_file(&site.lead.task, number);
        match crate::daemon::questions::read(&path).await {
            Ok(_) => {}
            Err(crate::daemon::questions::QuestionsProblem::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(source) => {
                return Err(PublishQuestionError::Read {
                    task: site.lead.task.clone(),
                    path,
                    source,
                }
                .into());
            }
        }
        self.ask_human_publish_failed(
            site,
            publication,
            "recovering an interrupted publication question",
            (tries, limit),
            now,
        )
        .await?;
        Ok(true)
    }

    /// Retries publication later, telling the human once if not yet announced.
    fn retry_publish(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        reason: &str,
        (tries, limit): (u32, u32),
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        tracing::warn!(
            event = LogEvent::GithubCallFailed.as_str(),
            team = %site.team,
            branch = %publication.head_branch,
            tries,
            limit,
            %reason,
            "a branch cannot be published: it is tried again in a minute"
        );
        let retry = Timestamp::from_utc(now.as_utc().saturating_add(PUBLISH_BACKOFF))?;
        let key = (site.team.clone(), publication.head_branch.clone());
        self.watch.retry_at.insert(key, retry);
        let announcement = Announcement::PublishFailed {
            team: site.team.clone(),
            branch: publication.head_branch.clone(),
        };
        if self.watch.announced.insert(announcement) {
            let values = Variables::new()
                .with("team", site.team.as_str())
                .and_then(|values| values.with("items", items_of(publication).join(", ")))
                .and_then(|values| values.with("reason", reason));
            self.tell_human(site.pulls, AutopilotNotice::PublishFailed, values, site.lead, now);
        }
        Ok(())
    }

    /// Tells the lead once that publication cannot succeed, releasing it.
    fn tell_lead_publish_failed(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        reason: &str,
        (tries, limit): (u32, u32),
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        tracing::warn!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = "publish_failed_lead",
            team = %site.team,
            branch = %publication.head_branch,
            tries,
            limit,
            %reason,
            "publication cannot succeed: telling the lead once"
        );
        let vars = Variables::new()
            .with("branch", publication.head_branch.as_str())
            .and_then(|v| v.with("reason", reason))
            .map_err(|source| PublishQuestionError::Variables {
                task: site.lead.task.clone(),
                source,
            })?;
        let text = site
            .pulls
            .texts
            .render(AutopilotPrompt::PublishFailed, &vars)
            .map_err(|source| PublishQuestionError::Render {
                task: site.lead.task.clone(),
                source,
            })?;
        self.release(&site.lead.task, Why::Gate(text.trim_end().to_owned()), look)
    }

    /// Prepares and writes the questions file for publication failure if not already present with answers.
    async fn prepare_publish_question_file(
        &self,
        site: Site<'_>,
        publication: &Publication,
        path: &std::path::Path,
        reason: &str,
    ) -> Result<(), PublishQuestionError> {
        let items_text = items_of(publication).join(", ");
        let vars = Variables::new()
            .with("task", site.lead.task.as_str())
            .and_then(|v| v.with("items", &items_text))
            .and_then(|v| v.with("branch", publication.head_branch.as_str()))
            .and_then(|v| v.with("reason", reason))
            .map_err(|source| PublishQuestionError::Variables {
                task: site.lead.task.clone(),
                source,
            })?;
        let text = site
            .pulls
            .texts
            .render(AutopilotPrompt::NoticePublishQuestion, &vars)
            .map_err(|source| PublishQuestionError::Render {
                task: site.lead.task.clone(),
                source,
            })?;
        match agent_runtime::text_file::create_new_text_file(path, &text, 0o600).await {
            Ok(()) => Ok(()),
            Err(agent_runtime::text_file::ReplaceError::Write { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                match crate::daemon::questions::read(path).await {
                    Ok(_) => Ok(()),
                    Err(source) => Err(PublishQuestionError::Read {
                        task: site.lead.task.clone(),
                        path: path.to_path_buf(),
                        source,
                    }),
                }
            }
            Err(source) => Err(PublishQuestionError::Write {
                task: site.lead.task.clone(),
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Asks the human once whether to retry publication or stop.
    async fn ask_human_publish_failed(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        reason: &str,
        (tries, limit): (u32, u32),
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        tracing::warn!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = "publish_failed_human",
            team = %site.team,
            branch = %publication.head_branch,
            tries,
            limit,
            %reason,
            "publication cannot succeed after telling the lead: asking the human once"
        );
        let member = self
            .parts
            .state
            .find_task(&site.lead.task)
            .map(|found| found.member)
            .ok_or_else(|| OwnerError::LostTask {
                task: site.lead.task.clone(),
            })?;
        let number = super::super::start::next_number(member).ok_or_else(|| OwnerError::QuestionNumbers {
            task: site.lead.task.clone(),
        })?;
        let path = self.parts.store.home().questions_file(&site.lead.task, number);

        self.prepare_publish_question_file(site, publication, &path, reason)
            .await?;
        let located = self.locate(&site.lead.task)?;
        let mut events = Vec::new();
        let notice = match self
            .ask(&site.lead.task, &located, now, &mut events)
            .await
            .map_err(|error| match error {
                OwnerError::Question { task, source } => OwnerError::from(PublishQuestionError::State { task, source }),
                other => other,
            })? {
            super::super::Asked::Recorded(notice) | super::super::Asked::Unusable(notice) => notice,
        };
        let item_ids = item_ids_of(publication);
        self.each_item(site.team, &item_ids, |item| {
            item.record_publish_question_branch(site.lead.task.clone(), number, now, publication.head_branch.clone());
            Ok(())
        })?;
        self.move_to(&site.lead.task, site.team, TaskStatus::Questions, &mut events)?;
        self.watch_task(&site.lead.task);
        self.commit(&site.lead.task, &located, now, events, Some(notice))
            .await?;
        self.adopt_saved_file(crate::daemon::questions::watch::Saved {
            task: site.lead.task.clone(),
            number,
        })
        .await
    }
}

#[cfg(test)]
#[path = "tests/joined_pass_readers.rs"]
mod joined_pass_readers_tests;

#[cfg(test)]
#[path = "publish/tests.rs"]
mod tests;
