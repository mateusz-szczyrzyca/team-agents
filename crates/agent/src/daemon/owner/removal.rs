//! `agent remove <team>`: takes a team out of the daemon with what is only its own. The unmerged branches are bundled
//! first and the team's open pull requests are closed; only then does the team leave the state, and its worktrees, task
//! directories and local branches are deleted. The history, the notifications and the branches on GitHub stay. A team
//! with a run going is refused unless the request says `force`, which ends its runs first.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::{AgentName, BranchName, TaskId, TeamId};
use agent_core::protocol::{Client, Failure, FailureKind, Removal, RemoveRequest, Target};
use agent_core::state::{ItemPhase, Team};
use agent_runtime::disk::directory_size;
use agent_runtime::git::{CommitIdentity, DeleteMode, Git, RemoveMode, Revision, TreeState};
use agent_runtime::github::{PullRequestUpdate, RepoSlug, StateChange};
use agent_runtime::process::error_chain;

use super::start::work_branch;
use super::{Owner, OwnerError};
use crate::daemon::control::Steer;

/// What the removal measures and decides before it changes anything.
struct Plan {
    /// The agent whose identity git runs with; the removal makes no commit, so it only has to be one of the team's.
    agent: AgentName,
    /// The checkout of the repository, where git runs for what the worktrees share. It is not one of the worktrees
    /// that are removed, so it outlives them. `None` when no worktree of the repository is left.
    checkout: Option<AbsolutePath>,
    /// The team's tasks that have a worktree on disk, with the directory of each.
    worktrees: Vec<(TaskId, AbsolutePath)>,
    /// The task directories of the team that exist, with their tasks.
    task_dirs: Vec<(TaskId, PathBuf)>,
    /// Every local branch the team works on or worked on.
    branches: BTreeSet<BranchName>,
    /// The branches that exist in the repository and so are deleted.
    existing: Vec<BranchName>,
    /// The existing branches with commits the base branch does not have: they go into the bundle.
    bundled: Vec<BranchName>,
    /// The worktrees with uncommitted tracked changes, which their removal takes with the changes.
    dirty: Vec<AbsolutePath>,
    /// The bytes of the worktrees and of the task directories, as measured before the removal.
    bytes: u64,
}

/// A failure of a removal step, with the cause in words for the human.
fn internal(what: &str, error: &dyn std::error::Error) -> Failure {
    Failure {
        kind: FailureKind::Internal,
        message: format!("{what}: {}", error_chain(error)),
    }
}

/// The refusal for a team that is not in the state.
fn not_found(team: &TeamId) -> Failure {
    Failure {
        kind: FailureKind::NotFound,
        message: format!("there is no team {team}"),
    }
}

/// Whether `path` exists. Only "not found" is an answer; any other failure to look is a failure of the removal.
fn exists(path: &Path) -> Result<bool, Failure> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(internal(&format!("cannot look at {}", path.display()), &error)),
    }
}

/// The path as an absolute path that git and the reply can name; a path that is not UTF-8 is refused, not replaced.
fn absolute(path: &Path) -> Result<AbsolutePath, Failure> {
    let text = path.to_str().ok_or_else(|| Failure {
        kind: FailureKind::Internal,
        message: format!("the path {} is not UTF-8 and cannot be named", path.display()),
    })?;
    AbsolutePath::from_str(text).map_err(|error| internal(&format!("the path {text}"), &error))
}

/// The tasks of a team: its crew and its reviewers.
fn tasks_of(team: &Team) -> Vec<TaskId> {
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task().clone())
        .chain(team.reviewers().iter().map(|member| member.task().clone()))
        .collect()
}

/// The checkout that git runs in for a worktree of a repository: the directory above the common git directory, which
/// the removal of a worktree does not touch; for a bare repository, the common directory itself.
async fn checkout_of(git: &Git) -> Result<AbsolutePath, Failure> {
    let common = git
        .common_dir()
        .await
        .map_err(|error| internal("cannot find the repository of a worktree", &error))?;
    let checkout = match common.parent() {
        Some(parent) if common.file_name() == Some(OsStr::new(".git")) => parent.to_path_buf(),
        _ => common,
    };
    absolute(&checkout)
}

impl Owner {
    /// Removes the team `request.team` and reports what it took away, bundled, closed and kept.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn remove_request(
        &mut self,
        client: &Client,
        request: RemoveRequest,
    ) -> Result<Result<Removal, Failure>, OwnerError> {
        let Some(team) = self.parts.state.team(&request.team) else {
            return Ok(Err(not_found(&request.team)));
        };
        let tasks = tasks_of(team);
        let running: Vec<TaskId> = tasks
            .iter()
            .filter(|task| self.in_flight.contains(*task))
            .cloned()
            .collect();
        let mut stopped = Vec::new();
        if !running.is_empty() {
            if !request.force {
                let names: Vec<String> = running.iter().map(ToString::to_string).collect();
                return Ok(Err(Failure {
                    kind: FailureKind::Conflict,
                    message: format!(
                        "team {} has a run going in {}: stop it with `agent stop`, or remove it with --force, which \
                         ends its runs first",
                        request.team,
                        names.join(", ")
                    ),
                }));
            }
            if let Err(failure) = self
                .control_request(client, Steer::Stop, &Target::Team(request.team.clone()))
                .await?
            {
                return Ok(Err(failure));
            }
            if tasks.iter().any(|task| self.in_flight.contains(task)) {
                return Ok(Err(Failure {
                    kind: FailureKind::Conflict,
                    message: format!(
                        "a run of team {} did not end after it was stopped: try again",
                        request.team
                    ),
                }));
            }
            stopped = running;
        }
        let plan = match self.removal_plan(&request.team).await {
            Ok(plan) => plan,
            Err(failure) => return Ok(Err(failure)),
        };
        let mut removal = Removal {
            team: request.team.clone(),
            stopped,
            tasks: tasks.clone(),
            bundle: None,
            bundled: plan.bundled.clone(),
            branches: Vec::new(),
            closed_pull_requests: Vec::new(),
            dirty_worktrees: plan.dirty.clone(),
            freed_bytes: plan.bytes,
            left_behind: Vec::new(),
        };
        if let Err(failure) = self.write_bundle(&request.team, &plan, &mut removal).await {
            return Ok(Err(failure));
        }
        if let Err(failure) = self.close_team_pulls(&request.team, &plan, &mut removal).await {
            return Ok(Err(failure));
        }
        // The team leaves the state before anything on disk goes: a daemon that stops in between has a team that is
        // gone and files left behind, never files gone under a team that the state still lists.
        if self.parts.state.remove_team(&request.team).is_none() {
            return Ok(Err(not_found(&request.team)));
        }
        self.forget_team(&request.team, &tasks);
        self.save().await?;
        self.delete_local(&plan, &mut removal).await;
        tracing::info!(
            event = agent_core::log::LogEvent::TeamChanged.as_str(),
            team = %request.team,
            tasks = tasks.len(),
            bundled = plan.bundled.len(),
            left_behind = removal.left_behind.len(),
            "team removed"
        );
        Ok(Ok(removal))
    }

    /// Measures and decides the removal of `team_id`; nothing is changed.
    async fn removal_plan(&self, team_id: &TeamId) -> Result<Plan, Failure> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Err(not_found(team_id));
        };
        let repo = team.repo().clone();
        let base = self
            .parts
            .settings
            .config
            .repo(&repo)
            .map(|config| config.base_branch.clone())
            .ok_or_else(|| Failure {
                kind: FailureKind::NotFound,
                message: format!("repository {repo} of team {team_id} is not in the configuration"),
            })?;
        let (_, first) = team.crew().roster().into_iter().next().ok_or_else(|| Failure {
            kind: FailureKind::Internal,
            message: format!("team {team_id} has no crew to take its identity from"),
        })?;
        let agent = AgentName::from_str(first.agent().name()).map_err(|error| internal("an agent name", &error))?;
        let home = self.parts.store.home();
        let mut worktrees = Vec::new();
        let mut task_dirs = Vec::new();
        let mut bytes: u64 = 0;
        for task in tasks_of(team) {
            let dir = home.worktree_dir(&repo, &task);
            if exists(&dir)? {
                let size = directory_size(&dir).map_err(|error| internal("cannot measure a worktree", &error))?;
                bytes = bytes.saturating_add(size);
                worktrees.push((task.clone(), absolute(&dir)?));
            }
            let dir = home.task_dir(&task);
            if exists(&dir)? {
                let size = directory_size(&dir).map_err(|error| internal("cannot measure a task directory", &error))?;
                bytes = bytes.saturating_add(size);
                task_dirs.push((task, dir));
            }
        }
        let mut branches = BTreeSet::new();
        for (_, member) in team.crew().roster() {
            let branch =
                work_branch(team, member).map_err(|error| internal("cannot name a branch of the team", &error))?;
            branches.insert(branch);
        }
        for progress in team.items() {
            match progress.phase() {
                ItemPhase::InProgress { branch, .. } => {
                    branches.insert(branch.clone());
                }
                ItemPhase::Fixing { branch, pr, .. } => {
                    branches.insert(branch.clone());
                    branches.insert(pr.branch.clone());
                }
                ItemPhase::InReview { pr, .. } | ItemPhase::Approved { pr, .. } => {
                    branches.insert(pr.branch.clone());
                }
                ItemPhase::Pending | ItemPhase::Done { .. } => {}
            }
        }
        let mut existing = Vec::new();
        let mut bundled = Vec::new();
        let mut dirty = Vec::new();
        let checkout = match worktrees.first() {
            Some((_, dir)) => Some(checkout_of(&self.git_at(dir, &agent)).await?),
            None => None,
        };
        if let Some(checkout) = &checkout {
            let git = self.git_at(checkout, &agent);
            for branch in &branches {
                let Some(tip) = git
                    .branch_tip(branch)
                    .await
                    .map_err(|error| internal("cannot read a branch", &error))?
                else {
                    continue;
                };
                existing.push(branch.clone());
                let in_base = git
                    .is_ancestor(&Revision::Commit(tip), &Revision::Branch(base.clone()))
                    .await
                    .map_err(|error| internal("cannot compare a branch with the base branch", &error))?;
                if !in_base {
                    bundled.push(branch.clone());
                }
            }
            for (_, dir) in &worktrees {
                match self
                    .git_at(dir, &agent)
                    .tree_state()
                    .await
                    .map_err(|error| internal("cannot read a worktree", &error))?
                {
                    TreeState::Dirty(_) => dirty.push(dir.clone()),
                    TreeState::Clean => {}
                }
            }
        }
        Ok(Plan {
            agent,
            checkout,
            worktrees,
            task_dirs,
            branches,
            existing,
            bundled,
            dirty,
            bytes,
        })
    }

    /// Git for `dir`, with the identity of `agent`.
    fn git_at(&self, dir: &AbsolutePath, agent: &AgentName) -> Git {
        Git::new(
            dir.clone(),
            CommitIdentity::for_agent(self.parts.settings.config.git(), agent),
        )
    }

    /// Writes the unmerged branches of the team into its bundle; nothing is written when every branch is in the base.
    ///
    /// # Errors
    /// [`Failure`] when the bundle cannot be written; the state and the files are then unchanged.
    async fn write_bundle(&self, team: &TeamId, plan: &Plan, removal: &mut Removal) -> Result<(), Failure> {
        if plan.bundled.is_empty() {
            return Ok(());
        }
        let Some(checkout) = &plan.checkout else {
            return Err(Failure {
                kind: FailureKind::Internal,
                message: format!("the unmerged branches of team {team} cannot be reached: no checkout is left"),
            });
        };
        let file = self.parts.store.home().removed_bundle(team);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| internal("cannot make the bundles directory", &error))?;
        }
        let bundle = absolute(&file)?;
        self.git_at(checkout, &plan.agent)
            .bundle_create(&bundle, &plan.bundled)
            .await
            .map_err(|error| internal("cannot write the bundle of the team", &error))?;
        removal.bundle = Some(bundle);
        Ok(())
    }

    /// Closes the open pull requests of the team's branches, each with a comment that says why. Without GitHub or a
    /// checkout nothing is closed, and the reply says so.
    ///
    /// # Errors
    /// [`Failure`] when GitHub refuses; the pull requests closed before it stay closed, and nothing else changed.
    async fn close_team_pulls(&self, team: &TeamId, plan: &Plan, removal: &mut Removal) -> Result<(), Failure> {
        if plan.branches.is_empty() {
            return Ok(());
        }
        let Some(pulls) = self.pulls.as_ref() else {
            removal
                .left_behind
                .push("open pull requests were not looked for: GitHub is not configured for the daemon".to_owned());
            return Ok(());
        };
        let Some(checkout) = &plan.checkout else {
            removal.left_behind.push(
                "open pull requests were not looked for: no worktree of the repository is left to read its origin"
                    .to_owned(),
            );
            return Ok(());
        };
        let origin = self
            .git_at(checkout, &plan.agent)
            .origin_url()
            .await
            .map_err(|error| internal("cannot read the origin of the repository", &error))?;
        let slug =
            RepoSlug::from_remote_url(&origin).map_err(|error| internal("the origin of the repository", &error))?;
        let github = pulls
            .client(slug)
            .map_err(|error| internal("cannot make the GitHub client", &error))?;
        for branch in &plan.branches {
            let Some(pr) = github
                .find_open_pull_request(branch)
                .await
                .map_err(|error| internal(&format!("cannot look for the pull request of {branch}"), &error))?
            else {
                continue;
            };
            let comment = format!(
                "Closed by `agent remove`: team {team} was removed from the team-agents daemon, and its unmerged work is \
                 kept on the host. The branch {branch} stays on GitHub."
            );
            github
                .post_comment(pr.number, &comment)
                .await
                .map_err(|error| internal(&format!("cannot comment on pull request {}", pr.number), &error))?;
            let update = PullRequestUpdate {
                title: None,
                body: None,
                base: None,
                state: Some(StateChange::Close),
            };
            github
                .update_pull_request(pr.number, &update)
                .await
                .map_err(|error| internal(&format!("cannot close pull request {}", pr.number), &error))?;
            removal.closed_pull_requests.push(pr.number);
        }
        Ok(())
    }

    /// Forgets what the owner keeps for the team and its tasks: their wake-ups, their holds, their queued messages.
    fn forget_team(&mut self, team: &TeamId, tasks: &[TaskId]) {
        self.looks.remove(team);
        self.closed.remove(team);
        self.outbox.retain(|outbound| outbound.team != *team);
        let gone = |task: &TaskId| tasks.contains(task);
        self.wakes.retain(|task, _| !gone(task));
        self.hangs.retain(|task, _| !gone(task));
        self.reasons.retain(|task, _| !gone(task));
        self.leads_being_told.retain(|task| !gone(task));
        self.lead_marks.retain(|task, _| !gone(task));
        self.wake_counts.retain(|task, _| !gone(task));
        self.wake_notes.retain(|task, _| !gone(task));
        self.serving.retain(|task, _| !gone(task));
        self.agent_waits.retain(|task| !gone(task));
        self.halts.retain(|task, _| !gone(task));
        self.holds.retain(|task, _| !gone(task));
        self.incomplete.retain(|task, _| !gone(task));
        self.watched.retain(|task| !gone(task));
        self.cancels.retain(|task, _| !gone(task));
        self.gate_stops.retain(|task, _| !gone(task));
    }

    /// Deletes the worktrees, the task directories and the local branches of a removed team. A step that fails is
    /// named in `removal.left_behind`; nothing is ever deleted before its commits are in the bundle or on GitHub.
    async fn delete_local(&self, plan: &Plan, removal: &mut Removal) {
        if let Some(checkout) = &plan.checkout {
            let git = self.git_at(checkout, &plan.agent);
            for (task, dir) in &plan.worktrees {
                if let Err(error) = git.worktree_remove(dir, RemoveMode::Force).await {
                    removal.left_behind.push(format!(
                        "worktree of {task} at {}: {}",
                        dir.as_path().display(),
                        error_chain(&error)
                    ));
                }
            }
            for branch in &plan.existing {
                match git.branch_delete(branch, DeleteMode::Force).await {
                    Ok(()) => removal.branches.push(branch.clone()),
                    Err(error) => removal
                        .left_behind
                        .push(format!("branch {branch}: {}", error_chain(&error))),
                }
            }
        } else if !plan.branches.is_empty() {
            removal
                .left_behind
                .push("branches not deleted: no worktree of the repository is left to reach them".to_owned());
        }
        for (task, dir) in &plan.task_dirs {
            if let Err(error) = std::fs::remove_dir_all(dir) {
                removal.left_behind.push(format!(
                    "task directory of {task} at {}: {}",
                    dir.display(),
                    error_chain(&error)
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
