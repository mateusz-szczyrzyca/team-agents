//! A team is not held behind a failed member whose work its lead's branch already holds.
//!
//! When the lead of a team ended `STATUS: DONE` with a passed gate at the tip of its branch, an item in progress behind a
//! member whose task failed is looked at once per look, before the branch is published ([`delivered_step`]): a member whose
//! branch the lead's contains and whose worktree holds nothing the lead was not told of is closed as taken over, and the
//! round is published; work the lead lacks is told to the lead once, with the commits and files, and when the lead ends
//! again without it the human is told, with the same list.

use std::str::FromStr;

use agent_core::decide::delivered::{Delivered, Delivery, Told, delivered_step};
use agent_core::ids::{AgentName, BranchName, ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::repo_path::RepoPath;
use agent_core::state::{CommitId, ItemPhase, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::git::{Git, Revision};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::super::briefing::BriefingProblem;
use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::{AutopilotNotice, Lead, PublishError, Pulls};
use crate::daemon::team::Why;

/// What the lead was told of a failed member's work, and when: the harness tells it once, and asks the human only when the
/// lead ended again without the commits.
#[derive(Debug, Clone)]
pub(super) struct ToldWork {
    /// When the lead was told.
    at: Timestamp,
    /// The commits the lead was told it lacks.
    commits: Vec<CommitId>,
    /// The uncommitted files the lead was told of.
    files: Vec<RepoPath>,
    /// Whether the human was told of what the lead left.
    asked: bool,
}

/// What the leads were told of the work of failed members, by the member's task. It is boxed in the watch: the owner is
/// held by value across awaits in the daemon's tests, and its size adds up in their futures.
#[derive(Debug, Default)]
pub(super) struct ToldWorks(std::collections::HashMap<TaskId, ToldWork>);

impl ToldWorks {
    /// What the lead was told of the work of the member `task`.
    pub(super) fn get(&self, task: &TaskId) -> Option<&ToldWork> {
        self.0.get(task)
    }

    /// What the lead was told of the work of the member `task`, to change.
    fn get_mut(&mut self, task: &TaskId) -> Option<&mut ToldWork> {
        self.0.get_mut(task)
    }

    /// Remembers that the lead was told `told` of the work of the member `task`.
    fn insert(&mut self, task: TaskId, told: ToldWork) {
        self.0.insert(task, told);
    }

    /// Forgets what the lead was told of the work of the member `task`.
    fn remove(&mut self, task: &TaskId) {
        self.0.remove(task);
    }
}

impl ToldWork {
    /// Whether the human was told of what the lead left; the notice of a standing item would say it a second time.
    pub(super) const fn asked(&self) -> bool {
        self.asked
    }
}

/// Why the work of a failed member cannot be compared with the lead's branch at this look.
#[derive(Debug, thiserror::Error)]
enum DeliveredError {
    /// A git call, a name or the gate record failed.
    #[error("cannot read the work of failed member {task}")]
    Read {
        /// The member's task.
        task: TaskId,
        /// What failed.
        source: PublishError,
    },
    /// The snapshot of the member's worktree could not be taken.
    #[error("cannot keep a snapshot of the worktree of failed member {task}")]
    Snapshot {
        /// The member's task.
        task: TaskId,
        /// What failed, boxed: a briefing problem is large.
        source: Box<BriefingProblem>,
    },
    /// The text for the lead or the human cannot be made.
    #[error("cannot make the text about the work of failed member {task}")]
    Text {
        /// The member's task.
        task: TaskId,
        /// What failed.
        source: PublishError,
    },
}

/// A member whose task failed while an item in progress is its own.
struct FailedHolder {
    /// The member's task.
    task: TaskId,
    /// The member's agent.
    agent: AgentName,
    /// The item that stands behind it.
    item: ItemId,
    /// The round of that item, which a snapshot is kept under.
    round: Option<RoundId>,
    /// The branches the member worked its items on.
    branches: Vec<BranchName>,
}

/// What a failed member has that the lead's branch lacks.
struct Lacking {
    /// The commits of the member's branches that are not in the lead's branch, oldest first.
    commits: Vec<CommitId>,
    /// The files the member's worktree holds that no commit does.
    files: Vec<RepoPath>,
}

impl Owner {
    /// The members of `team` whose task failed and whose item is in progress, once each.
    fn failed_holders(&self, team_id: &TeamId) -> Vec<FailedHolder> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Vec::new();
        };
        let roster = team.crew().roster();
        let mut holders: Vec<FailedHolder> = Vec::new();
        for progress in team.items() {
            let round = team.round_of(progress.id());
            if round.is_some_and(|round| round.delivered_by().is_some()) {
                continue;
            }
            let ItemPhase::InProgress { assignee, branch } = progress.phase() else {
                continue;
            };
            let Some((_, member)) = roster.iter().find(|(role, member)| {
                *role == Role::Member
                    && member.status() == TaskStatus::Failed
                    && member.agent().name() == assignee.as_str()
            }) else {
                continue;
            };
            let Ok(agent) = agent_name_of(member.agent().name()) else {
                continue;
            };
            match holders.iter_mut().find(|holder| holder.task == *member.task()) {
                Some(holder) => holder.branches.push(branch.clone()),
                None => holders.push(FailedHolder {
                    task: member.task().clone(),
                    agent,
                    item: progress.id(),
                    round: round.map(|round| round.id().clone()),
                    branches: vec![branch.clone()],
                }),
            }
        }
        holders
    }

    /// Looks at the failed members of `team_id` once the lead is done with a passed gate, and releases, tells the lead or tells
    /// the human as [`delivered_step`] decides. A member whose work cannot be read at this look is left as it is and
    /// logged: the round waits as it did before this step, and the next look tries again.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be changed or saved.
    pub(super) async fn settle_failed_members(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team_id)?;
        let lead_waits = self
            .parts
            .state
            .find_task(&lead.task)
            .is_some_and(|found| found.member.status() == TaskStatus::Waiting);
        if !lead.done || !lead_waits {
            return Ok(());
        }
        let holders = self.failed_holders(team_id);
        if holders.is_empty() {
            return Ok(());
        }
        let (lead_git, tip, gate_passed) = match self.lead_gate(&lead).await {
            Ok(found) => found,
            Err(error) => {
                // The round keeps waiting, as before this step existed; the next look reads the gate again.
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    team = %team_id,
                    task = %lead.task,
                    error = %error_chain(&error),
                    "the gate of a lead that is done cannot be read: failed members are not looked at"
                );
                return Ok(());
            }
        };
        for holder in holders {
            let lacking = match self.lacking_of(&holder, &lead_git, &tip).await {
                Ok(lacking) => lacking,
                Err(error) => {
                    // The member stays failed and holds the round, as before; the next look tries again.
                    tracing::warn!(
                        event = LogEvent::GitCallFailed.as_str(),
                        team = %team_id,
                        task = %holder.task,
                        error = %error_chain(&error),
                        "the work of a failed member cannot be compared with its lead's branch"
                    );
                    continue;
                }
            };
            let step = self.step_for(&lead, &holder, &lacking, gate_passed);
            match step {
                Delivered::Nothing => {}
                Delivered::Release => self.release_member(team_id, &holder).await?,
                Delivered::TellLead => self.tell_lead_of(team_id, &lead, &holder, lacking, look).await?,
                Delivered::AskHuman => self.ask_human_of(pulls, team_id, &lead, &holder, &lacking)?,
            }
        }
        Ok(())
    }

    /// The lead's git handle, the tip of its branch, and whether its gate passed on a clean tree there.
    async fn lead_gate(&self, lead: &Lead) -> Result<(Git, CommitId, bool), PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let tip = git.rev_parse(&Revision::Head).await?;
        let record = self.parts.store.load_gate_record(&lead.task).await?;
        let passed = record.is_some_and(|record| record.result.head == tip && record.counts_as_pass());
        Ok((git, tip, passed))
    }

    /// What the member of `holder` has that the lead's branch, at `tip`, lacks.
    async fn lacking_of(&self, holder: &FailedHolder, lead_git: &Git, tip: &CommitId) -> Result<Lacking, PublishError> {
        let repo = self.repo_of(&holder.task)?;
        let member_git = self.worktree_git(&repo, &holder.task, &holder.agent)?;
        let mut commits: Vec<CommitId> = Vec::new();
        for branch in &holder.branches {
            // A branch that is gone has no commit to lack.
            if let Some(branch_tip) = member_git.branch_tip(branch).await? {
                for record in lead_git.range_log(tip, &branch_tip).await? {
                    if !commits.contains(&record.id) {
                        commits.push(record.id);
                    }
                }
            }
        }
        let files = member_git
            .uncommitted_changes()
            .await?
            .into_iter()
            .map(|change| change.path)
            .collect();
        Ok(Lacking { commits, files })
    }

    /// What [`delivered_step`] says of `holder`'s work.
    fn step_for(&self, lead: &Lead, holder: &FailedHolder, lacking: &Lacking, gate_passed: bool) -> Delivered {
        let told = self.watch.delivered.get(&holder.task);
        let answered = told.is_some_and(|told| {
            self.parts
                .state
                .find_task(&lead.task)
                .and_then(|found| found.member.runs().last().map(agent_core::state::Run::started_at))
                .is_some_and(|started| started.as_utc() > told.at.as_utc())
        });
        delivered_step(&Delivery {
            member: TaskStatus::Failed,
            lead_done: lead.done,
            gate_passed,
            commits: &lacking.commits,
            files: &lacking.files,
            told: told.map(|told| Told {
                commits: &told.commits,
                files: &told.files,
                answered,
            }),
            asked: told.is_some_and(|told| told.asked),
        })
    }

    /// The repository of the team the task `task` is in.
    fn repo_of(&self, task: &TaskId) -> Result<agent_core::config::RepoName, PublishError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| PublishError::Owner(Box::new(OwnerError::LostTask { task: task.clone() })))?;
        Ok(found.team.repo().clone())
    }

    /// Closes the failed member's task as taken over by its lead: the lead's branch holds its work, so the item goes with
    /// the lead's delivery, as for a member the human stopped.
    async fn release_member(&mut self, team_id: &TeamId, holder: &FailedHolder) -> Result<(), OwnerError> {
        let mut events = Vec::new();
        self.move_to(&holder.task, team_id, TaskStatus::Stopped, &mut events)?;
        self.save().await?;
        let at = Timestamp::now(&self.parts.clock)?;
        for kind in events {
            self.publish(at, kind);
        }
        self.watch.delivered.remove(&holder.task);
        log_decision(
            "takeover",
            &format!("task={}, from={}", holder.task, holder.agent),
            "lead, its branch holds the work",
        );
        tracing::info!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = "failed_member_delivered",
            team = %team_id,
            task = %holder.task,
            item = %holder.item,
            "the lead's branch holds the work of a failed member: the member is closed as taken over"
        );
        Ok(())
    }

    /// Tells the lead what the failed member has that its branch lacks, once, and remembers it.
    async fn tell_lead_of(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        holder: &FailedHolder,
        lacking: Lacking,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        let snapshot = if lacking.files.is_empty() {
            "none: nothing is uncommitted".to_owned()
        } else {
            match self.keep_snapshot(holder).await {
                Ok(reference) => format!("`{reference}`"),
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::SnapshotFailed.as_str(),
                        team = %team_id,
                        task = %holder.task,
                        error = %error_chain(&error),
                        "the worktree of a failed member cannot be snapshotted: its lead is not told yet"
                    );
                    return Ok(());
                }
            }
        };
        let text = self.lacks_text(&pulls, holder, &lacking, &snapshot).map_err(|error| {
            tracing::error!(
                event = LogEvent::NotificationFailed.as_str(),
                team = %team_id,
                error = %error_chain(&error),
                "what a lead is told about a failed member's work cannot be made"
            );
        });
        let Ok(text) = text else {
            return Ok(());
        };
        tracing::info!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = "failed_member_work_lacks",
            team = %team_id,
            task = %lead.task,
            member = %holder.task,
            commits = lacking.commits.len(),
            files = lacking.files.len(),
            "a failed member has work its lead's branch lacks: the lead is told, once"
        );
        let at = Timestamp::now(&self.parts.clock)?;
        self.watch.delivered.insert(
            holder.task.clone(),
            ToldWork {
                at,
                commits: lacking.commits,
                files: lacking.files,
                asked: false,
            },
        );
        self.release(&lead.task, Why::Gate(text.trim_end().to_owned()), look)
    }

    /// Keeps a snapshot of the failed member's worktree under the round of its item, and returns its reference.
    async fn keep_snapshot(&mut self, holder: &FailedHolder) -> Result<String, DeliveredError> {
        let snapshot_error = |source| DeliveredError::Snapshot {
            task: holder.task.clone(),
            source: Box::new(source),
        };
        let repo = self.repo_of(&holder.task).map_err(|source| DeliveredError::Read {
            task: holder.task.clone(),
            source,
        })?;
        let round = holder.round.clone().ok_or_else(|| {
            snapshot_error(BriefingProblem::NoTask {
                task: holder.task.clone(),
            })
        })?;
        let path = self.parts.store.home().worktree_dir(&repo, &holder.task);
        let text = path.to_str().ok_or_else(|| {
            snapshot_error(BriefingProblem::PathText {
                task: holder.task.clone(),
            })
        })?;
        let worktree = agent_core::config::AbsolutePath::from_str(text).map_err(|source| {
            snapshot_error(BriefingProblem::Path {
                task: holder.task.clone(),
                source,
            })
        })?;
        self.snapshot(&worktree, &round, &holder.task)
            .await
            .map_err(snapshot_error)
    }

    /// What the lead is told about the work `lacking` of the member of `holder`.
    fn lacks_text(
        &self,
        pulls: &Pulls,
        holder: &FailedHolder,
        lacking: &Lacking,
        snapshot: &str,
    ) -> Result<String, DeliveredError> {
        let repo = self.repo_of(&holder.task).map_err(|source| DeliveredError::Read {
            task: holder.task.clone(),
            source,
        })?;
        let worktree = self.parts.store.home().worktree_dir(&repo, &holder.task);
        let branch = holder.branches.first().map_or_else(String::new, ToString::to_string);
        Variables::new()
            .with("member", holder.agent.as_str())
            .and_then(|values| values.with("branch", branch))
            .and_then(|values| values.with("worktree", worktree.display().to_string()))
            .and_then(|values| values.with("commits", list_of(&lacking.commits)))
            .and_then(|values| values.with("files", list_of(&lacking.files)))
            .and_then(|values| values.with("snapshot", snapshot))
            .map_err(PublishError::from)
            .and_then(|values| Ok(pulls.texts.render(AutopilotPrompt::MemberWorkLacks, &values)?))
            .map_err(|source| DeliveredError::Text {
                task: holder.task.clone(),
                source,
            })
    }

    /// Tells the human, once, what the lead left of a failed member's work.
    fn ask_human_of(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        holder: &FailedHolder,
        lacking: &Lacking,
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let values = Variables::new()
            .with("team", team_id.as_str())
            .and_then(|values| values.with("item", holder.item.to_string()))
            .and_then(|values| values.with("member", holder.agent.as_str()))
            .and_then(|values| values.with("task", holder.task.as_str()))
            .and_then(|values| values.with("commits", list_of(&lacking.commits)))
            .and_then(|values| values.with("files", list_of(&lacking.files)));
        self.tell_human(pulls, AutopilotNotice::MemberWorkLeft, values, lead, now);
        if let Some(told) = self.watch.delivered.get_mut(&holder.task) {
            told.asked = true;
        }
        Ok(())
    }
}

/// `items` as the lines of a list, `none` for an empty one.
fn list_of<T: std::fmt::Display>(items: &[T]) -> String {
    if items.is_empty() {
        return "- none".to_owned();
    }
    items
        .iter()
        .map(|item| format!("- `{item}`"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The agent name `name` of a member of the state.
fn agent_name_of(name: &str) -> Result<AgentName, agent_core::ids::InvalidName> {
    name.parse()
}
