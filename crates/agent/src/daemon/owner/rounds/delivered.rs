//! A team is not held behind a failed member whose work its lead's branch already holds.
//!
//! When the lead of a team ended `STATUS: DONE` with a passed gate at the tip of its branch, an item in progress behind a
//! member whose task failed or was stopped is looked at once per look, before the branch is published
//! ([`delivered_step`]): a failed member whose branch the lead's contains and whose worktree holds nothing the lead was not
//! told of is closed as taken over, and the round is published; work the lead lacks is told to the lead once — in one
//! briefing for all members of the look —, with the commits and files, and when the lead ends again without it the human
//! is told, with the same list. What the lead was told is kept in memory and is told again after a restart of the daemon,
//! never dropped; what the human was told is found among the saved notices, so the human is asked once.

use agent_core::history::CheckpointCause;
use std::path::Path;
use std::str::FromStr;

use agent_core::decide::delivered::{Delivered, Delivery, Told, delivered_step};
use agent_core::ids::{AgentName, BranchName, ItemId, RoundId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::repo_path::RepoPath;
use agent_core::state::{CommitId, ItemPhase, ItemProgress, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::git::{Git, Revision};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::super::briefing::BriefingProblem;
use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::asleep::{SavedNoticeError, saved_notice_where};
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
    /// The tree the member's worktree held when the lead was told of its files: a file whose bytes changed since is not the
    /// file the lead was told of. `None` when the worktree held no uncommitted file.
    tree: Option<String>,
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

/// A member whose task failed after a run that called a tool, or was stopped, while an item in progress is its own.
struct FailedHolder {
    /// The member's task.
    task: TaskId,
    /// The member's agent.
    agent: AgentName,
    /// How the member's task ended: failed, or stopped by the human.
    status: TaskStatus,
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
    /// The tree the member's worktree holds now; `None` when no file is uncommitted.
    tree: Option<String>,
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
                    && matches!(member.status(), TaskStatus::Failed | TaskStatus::Stopped)
                    && member.agent().name() == assignee.as_str()
            }) else {
                continue;
            };
            // A member that failed without ever calling a tool did no work the lead could hold: its failure is a setup defect
            // that stays with the human, as the brief of the ladder says (`used_tool` is what the daemon recorded of its
            // runs). A member the human stopped is looked at whatever it did: its files are never dropped.
            if member.status() == TaskStatus::Failed && !member.runs().iter().any(agent_core::state::Run::used_tool) {
                continue;
            }
            // A stopped member the ladder gave to the lead had its files listed and snapshotted for the lead: the lead was
            // told, and its work is the lead's. A member that was only started in a new session before the human stopped it
            // was not given to anyone, and is looked at like any other.
            if member.status() == TaskStatus::Stopped && member.given_to_lead() {
                continue;
            }
            let Ok(agent) = agent_name_of(member.agent().name()) else {
                continue;
            };
            match holders.iter_mut().find(|holder| holder.task == *member.task()) {
                Some(holder) => holder.branches.push(branch.clone()),
                None => holders.push(FailedHolder {
                    task: member.task().clone(),
                    agent,
                    status: member.status(),
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
        let (lead_git, tip, gate_passed) = match self.lead_gate(team_id, &lead).await {
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
        let mut tell: Vec<(FailedHolder, Lacking)> = Vec::new();
        for holder in holders {
            let lacking = match self.lacking_of(&holder, &lead_git, &tip).await {
                Ok(lacking) => lacking,
                Err(error) => {
                    // The member stays as it is and the next look tries again.
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
            self.remember_saved_question(team_id, &holder, &lacking).await?;
            let step = self.step_for(&lead, &holder, &lacking, gate_passed);
            match step {
                Delivered::Nothing => {}
                Delivered::Release => self.release_member(team_id, &holder).await?,
                Delivered::TellLead => tell.push((holder, lacking)),
                Delivered::AskHuman => self.ask_human_of(pulls, team_id, &lead, &holder, &lacking)?,
            }
        }
        self.tell_lead_of(team_id, &lead, tell, look).await
    }

    /// Finds the question to the human about `holder`'s work among the saved notices of a daemon that ran before, when this
    /// daemon has not told the lead of it yet, and remembers that the human was asked: the human is asked once, also when
    /// the daemon restarted and the lead ends again.
    async fn remember_saved_question(
        &mut self,
        team_id: &TeamId,
        holder: &FailedHolder,
        lacking: &Lacking,
    ) -> Result<(), OwnerError> {
        if lacking.commits.is_empty() || self.watch.delivered.get(&holder.task).is_some() {
            return Ok(());
        }
        let directory = self.notice_directory();
        let asked = member_work_notice_saved(
            &directory,
            team_id,
            holder.agent.as_str(),
            &holder.task,
            &lacking.commits,
        )
        .await;
        match asked {
            Ok(true) => {
                let at = Timestamp::now(&self.parts.clock)?;
                self.watch.delivered.insert(
                    holder.task.clone(),
                    ToldWork {
                        at,
                        commits: lacking.commits.clone(),
                        files: lacking.files.clone(),
                        tree: lacking.tree.clone(),
                        asked: true,
                    },
                );
            }
            Ok(false) => {}
            Err(error) => {
                // A failed lookup must not hide work that needs the human: the lead is told, and the human asked again.
                tracing::warn!(
                    event = LogEvent::NotificationFailed.as_str(),
                    team = %team_id,
                    task = %holder.task,
                    error = %error_chain(&error),
                    "saved notices cannot be inspected: the human may be asked of the work of a failed member again"
                );
            }
        }
        Ok(())
    }

    /// The lead's git handle, the tip of the branch of the item it works on (what its worktree has out when it has no such
    /// item), and whether its gate passed on a clean tree there.
    async fn lead_gate(&self, team_id: &TeamId, lead: &Lead) -> Result<(Git, CommitId, bool), PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let branch = self.parts.state.team(team_id).and_then(|team| {
            crate::daemon::owner::item_branch::target_branch(
                team.items().iter().map(ItemProgress::phase),
                lead.agent.as_str(),
            )
            .cloned()
        });
        let tip = match branch {
            Some(branch) => self.item_tip(lead, &branch).await?.tip,
            None => git.rev_parse(&Revision::Head).await?,
        };
        let record = self.parts.store.load_gate_record(&lead.task).await?;
        let passed = record.is_some_and(|record| record.result.head == tip && record.counts_as_pass());
        Ok((git, tip, passed))
    }

    /// What the member of `holder` has that the lead's branch, at `tip`, lacks.
    async fn lacking_of(&self, holder: &FailedHolder, lead_git: &Git, tip: &CommitId) -> Result<Lacking, PublishError> {
        let mut commits: Vec<CommitId> = Vec::new();
        for branch in &holder.branches {
            // A branch that is gone has no commit to lack.
            if let Some(branch_tip) = lead_git.branch_tip(branch).await? {
                for record in lead_git.range_log(tip, &branch_tip).await? {
                    if !commits.contains(&record.id) {
                        commits.push(record.id);
                    }
                }
            }
        }
        let repo = self.repo_of(&holder.task)?;
        // A worktree that was removed holds no uncommitted file.
        if !self.parts.store.home().worktree_dir(&repo, &holder.task).is_dir() {
            return Ok(Lacking {
                commits,
                files: Vec::new(),
                tree: None,
            });
        }
        let member_git = self.worktree_git(&repo, &holder.task, &holder.agent)?;
        let files: Vec<RepoPath> = member_git
            .uncommitted_changes()
            .await?
            .into_iter()
            .map(|change| change.path)
            .collect();
        let tree = if files.is_empty() {
            None
        } else {
            Some(member_git.worktree_tree().await?)
        };
        Ok(Lacking { commits, files, tree })
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
            member: holder.status,
            lead_done: lead.done,
            gate_passed,
            commits: &lacking.commits,
            files: &lacking.files,
            told: told.map(|told| Told {
                commits: &told.commits,
                // A file whose bytes changed since the lead was told of it is not the file it was told of.
                files: if told.tree == lacking.tree { &told.files } else { &[] },
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

    /// Tells the lead what the members of `pending` have that its branch lacks: one briefing for all of them, so that no
    /// member's list replaces another's, and each is remembered as told only when the briefing is the lead's reason for
    /// its next run.
    async fn tell_lead_of(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        pending: Vec<(FailedHolder, Lacking)>,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        let mut texts: Vec<String> = Vec::new();
        let mut included: Vec<(FailedHolder, Lacking)> = Vec::new();
        for (holder, lacking) in pending {
            let snapshot = if lacking.files.is_empty() {
                "none: nothing is uncommitted".to_owned()
            } else {
                match self.keep_snapshot(&holder).await {
                    Ok(reference) => format!("`{reference}`"),
                    Err(error) => {
                        tracing::warn!(
                            event = LogEvent::SnapshotFailed.as_str(),
                            team = %team_id,
                            task = %holder.task,
                            error = %error_chain(&error),
                            "the worktree of a failed member cannot be snapshotted: its lead is not told yet"
                        );
                        continue;
                    }
                }
            };
            match self.lacks_text(&pulls, &holder, &lacking, &snapshot) {
                Ok(text) => {
                    texts.push(text.trim_end().to_owned());
                    included.push((holder, lacking));
                }
                Err(error) => tracing::error!(
                    event = LogEvent::NotificationFailed.as_str(),
                    team = %team_id,
                    error = %error_chain(&error),
                    "what a lead is told about a failed member's work cannot be made"
                ),
            }
        }
        if included.is_empty() {
            return Ok(());
        }
        let text = texts.join("\n\n");
        let at = Timestamp::now(&self.parts.clock)?;
        self.release(&lead.task, Why::Gate(text.clone()), look)?;
        // The lead waits, so the briefing is now what its next run starts with; a lead that was not queued by it is told
        // again at the next look.
        if !matches!(self.reasons.get(&lead.task), Some(Why::Gate(queued)) if *queued == text) {
            tracing::warn!(
                event = LogEvent::ChecksDecided.as_str(),
                outcome = "failed_member_work_not_told",
                team = %team_id,
                task = %lead.task,
                "the lead could not be given what a failed member has that its branch lacks: it is told at the next look"
            );
            return Ok(());
        }
        for (holder, lacking) in included {
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
            self.watch.delivered.insert(
                holder.task.clone(),
                ToldWork {
                    at,
                    commits: lacking.commits,
                    files: lacking.files,
                    tree: lacking.tree,
                    asked: false,
                },
            );
        }
        Ok(())
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
            .and_then(|values| values.with("files", list_of(&lacking.files)))
            .and_then(|values| values.with("outcome", outcome_of(holder.status)));
        self.tell_human(pulls, AutopilotNotice::MemberWorkLeft, values, lead, now);
        self.queue_for_task(&holder.task, CheckpointCause::WaitsForHuman);
        if let Some(told) = self.watch.delivered.get_mut(&holder.task) {
            told.asked = true;
        }
        Ok(())
    }
}

/// Whether the human was asked, in a saved notice of `directory`, of the work of `member` (task `task`) of `team` that the
/// lead left, naming each of `commits`.
///
/// # Errors
/// [`SavedNoticeError`] when the saved notices cannot be read.
pub(super) async fn member_work_notice_saved(
    directory: &Path,
    team: &TeamId,
    member: &str,
    task: &TaskId,
    commits: &[CommitId],
) -> Result<bool, SavedNoticeError> {
    // The words of `notice-member-work-left.md` that name the team, the member and its task.
    let title = format!("Team {team} holds item");
    let who = format!("member {member} (task {task})");
    saved_notice_where(directory, |text| {
        text.contains(&title) && text.contains(&who) && commits.iter().all(|commit| text.contains(&commit.to_string()))
    })
    .await
}

/// What happens to the item of a member with `status` whose work the lead left: a failed member keeps holding it, and the
/// item of a stopped one goes with the lead's delivery.
fn outcome_of(status: TaskStatus) -> &'static str {
    if status == TaskStatus::Failed {
        "The item is not published, so that the work is not dropped without a decision."
    } else {
        "The item is published with the lead's branch; the work stays on the member's branch and in its worktree."
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
