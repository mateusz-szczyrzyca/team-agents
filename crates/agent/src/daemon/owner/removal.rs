//! `agent remove <team>`: takes a team out of the daemon with what is only its own. The unmerged branches are bundled
//! first and the team's open pull requests are closed; only then does the team leave the state, and its worktrees, task
//! directories and local branches are deleted. The history, the notifications and the branches on GitHub stay. A team
//! with a run going is refused unless the request says `force`, which ends its runs first.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::{AgentName, BranchName, TaskId, TeamId};
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::protocol::{Client, Failure, FailureKind, Removal, RemoveRequest, Reply, Target};
use agent_core::state::CommitId;
use agent_core::state::{ItemPhase, Team, WakeDelivery};
use agent_runtime::disk::directory_size;
use agent_runtime::git::{CommitIdentity, DeleteMode, Git, RemoveMode, Revision, StatusEntry, TreeState};
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
    /// The team's tasks that have a worktree on disk, with the directory of each and its bytes.
    worktrees: Vec<(TaskId, AbsolutePath, u64)>,
    /// The task directories of the team that exist, with their tasks and bytes.
    task_dirs: Vec<(TaskId, PathBuf, u64)>,
    /// Every local branch the team works on or worked on.
    branches: BTreeSet<BranchName>,
    /// The branches that exist in the repository and so are deleted.
    existing: Vec<BranchName>,
    /// The existing branches with commits the base branch does not have: they go into the bundle.
    bundled: Vec<BranchName>,
    /// The worktrees with uncommitted tracked changes, whether cleanup later removes them or leaves them behind.
    dirty: Vec<AbsolutePath>,
    /// Worktree HEAD commits absent from the base, whether or not a team branch also holds them.
    /// Each is kept under `refs/agent-removed/<task>` in the bundle before the worktree is removed.
    heads: Vec<(TaskId, CommitId)>,
    /// What the measuring found that the reply must name, such as a worktree that is not a git checkout.
    notes: Vec<String>,
}

/// What the removal measures on disk: the worktrees and the task directories that exist, each with its bytes.
struct Measure {
    /// The team's tasks that have a worktree on disk, with the directory of each and its bytes.
    worktrees: Vec<(TaskId, AbsolutePath, u64)>,
    /// The task directories of the team that exist, with their tasks and bytes.
    task_dirs: Vec<(TaskId, PathBuf, u64)>,
}

/// A failure of a removal step, with the cause in words for the human.
pub(super) fn internal(what: &str, error: &dyn std::error::Error) -> Failure {
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

/// Whether a changed path of a worktree is one git does not track: `??` in the status.
fn is_untracked(entry: &StatusEntry) -> bool {
    entry.index == '?' && entry.worktree == '?'
}

/// Reads tracked changes and HEAD commits of Git worktrees, skipping plain directories.
/// HEAD commits absent from the base are kept even when a team branch also holds them.
async fn inspect_worktrees(
    owner: &Owner,
    worktrees: &[(TaskId, AbsolutePath, u64)],
    git: &Git,
    base: &BranchName,
    agent: &AgentName,
) -> Result<(Vec<AbsolutePath>, Vec<(TaskId, CommitId)>), Failure> {
    let mut dirty = Vec::new();
    let mut heads = Vec::new();
    for (task, dir, _) in worktrees {
        if !exists(&dir.as_path().join(".git")).await? {
            continue;
        }
        let handle = owner.git_at(dir, agent);
        match handle
            .tree_state()
            .await
            .map_err(|error| internal("cannot read a worktree", &error))?
        {
            TreeState::Dirty(entries) if entries.iter().any(|entry| !is_untracked(entry)) => {
                dirty.push(dir.clone());
            }
            // Untracked files alone are not changes of the work: git keeps no record of them.
            TreeState::Dirty(_) | TreeState::Clean => {}
        }
        let head = handle
            .rev_parse(&Revision::Head)
            .await
            .map_err(|error| internal("cannot read the HEAD of a worktree", &error))?;
        let in_base = git
            .is_ancestor(&Revision::Commit(head.clone()), &Revision::Branch(base.clone()))
            .await
            .map_err(|error| internal("cannot compare a HEAD with the base branch", &error))?;
        if !in_base {
            heads.push((task.clone(), head));
        }
    }
    Ok((dirty, heads))
}

/// The reference under which the bundle keeps the HEAD commit of a worktree of `task`.
fn head_ref(task: &TaskId) -> String {
    format!("refs/agent-removed/{task}")
}

/// The bytes under `path`, walked on the blocking pool.
async fn size_of(path: PathBuf) -> Result<u64, Failure> {
    let measured = tokio::task::spawn_blocking(move || directory_size(&path))
        .await
        .map_err(|error| internal("the measure of a directory stopped", &error))?;
    measured.map_err(|error| internal("cannot measure a directory", &error))
}

/// Removes the tree under `path` on the blocking pool; the error is the file system's, or the failure to run the
/// removal at all.
async fn remove_tree(path: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&path))
        .await
        .map_err(std::io::Error::other)?
}

/// Whether `path` exists. Only "not found" is an answer; any other failure to look is a failure of the removal.
pub(super) async fn exists(path: &Path) -> Result<bool, Failure> {
    match tokio::fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(internal(&format!("cannot look at {}", path.display()), &error)),
    }
}

/// The path as an absolute path that git and the reply can name; a path that is not UTF-8 is refused, not replaced.
pub(super) fn absolute(path: &Path) -> Result<AbsolutePath, Failure> {
    let text = path.to_str().ok_or_else(|| Failure {
        kind: FailureKind::Internal,
        message: format!("the path {} is not UTF-8 and cannot be named", path.display()),
    })?;
    AbsolutePath::from_str(text).map_err(|error| internal(&format!("the path {text}"), &error))
}

/// The tasks of a team: its crew and its reviewers.
pub(super) fn tasks_of(team: &Team) -> Vec<TaskId> {
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task().clone())
        .chain(team.reviewers().iter().map(|member| member.task().clone()))
        .collect()
}

/// Every local branch the team works on or worked on, as the state names them.
pub(super) fn branches_of(team: &Team) -> Result<BTreeSet<BranchName>, Failure> {
    let mut branches = BTreeSet::new();
    for (_, member) in team.crew().roster() {
        let branch = work_branch(team, member).map_err(|error| internal("cannot name a branch of the team", &error))?;
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
    Ok(branches)
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
    /// The answer to `remove`: what the removal took away, or the refusal.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn remove_reply(
        &mut self,
        client: &Client,
        request: RemoveRequest,
    ) -> Result<Result<Reply, Failure>, OwnerError> {
        Ok(self
            .remove_request(client, request)
            .await?
            .map(|removal| Reply::Removed {
                removal: Box::new(removal),
            }))
    }

    /// Removes the team `request.team` and reports what it took away, bundled, closed and kept.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved.
    pub(super) async fn remove_request(
        &mut self,
        client: &Client,
        request: RemoveRequest,
    ) -> Result<Result<Removal, Failure>, OwnerError> {
        self.remove_for(client, request, "requested by the human").await
    }

    /// Uses the command safeguards while recording the caller's reason.
    pub(super) async fn remove_for(
        &mut self,
        client: &Client,
        request: RemoveRequest,
        reason: &str,
    ) -> Result<Result<Removal, Failure>, OwnerError> {
        let Some(team) = self.parts.state.team(&request.team) else {
            return Ok(Err(not_found(&request.team)));
        };
        let tasks = tasks_of(team);
        while let Some(joined) = self.gate_answers.try_join_next_with_id() {
            self.gate_answer_done(joined)?;
        }
        let running: Vec<TaskId> = tasks
            .iter()
            .filter(|task| {
                self.in_flight.contains(*task)
                    || self.parked.contains_key(*task)
                    || self.request_checking.values().any(|checking| checking.task() == *task)
                    || self.gate_answer_tasks.values().any(|owner| owner == *task)
            })
            .cloned()
            .collect();
        let stopped = match self.settle_removal(client, &request, &tasks, running).await? {
            Ok(stopped) => stopped,
            Err(failure) => return Ok(Err(failure)),
        };
        while tasks.iter().any(|task| self.pending_removals.contains(task)) {
            if let Some(joined) = self.disk_jobs.join_next_with_id().await {
                self.disk_job_done(joined).await;
            }
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
            freed_bytes: 0,
            left_behind: plan.notes.clone(),
        };
        if let Err(failure) = self.write_bundle(&request.team, &plan, &mut removal).await {
            return Ok(Err(failure));
        }
        if let Err(failure) = self.close_team_pulls(&request.team, &plan, &mut removal).await {
            return Ok(Err(failure));
        }
        for task in &tasks {
            if let Some(delivery) = self.parts.state.wake_delivery(task) {
                let (phase, message, count) = match delivery {
                    WakeDelivery::Pending { message, count } => ("pending", message, count),
                    WakeDelivery::Delivered { message, count } => ("delivered", message, count),
                };
                let message = message.map_or_else(|| "unknown".to_owned(), |message| message.to_string());
                removal.left_behind.push(format!(
                    "removed wake budget: task {task}, message {message}, count {count}, state {phase}, source {}",
                    self.parts.store.home().state_file().display()
                ));
            }
        }
        // The team leaves the state before anything on disk goes: a daemon that stops in between has a team that is
        // gone and files left behind, never files gone under a team that the state still lists.
        let mut next = self.parts.state.clone();
        if next.remove_team(&request.team).is_none() {
            return Ok(Err(not_found(&request.team)));
        }
        self.parts.store.save(&next).await?;
        self.parts.state = next;
        self.forget_team(&request.team, &tasks);
        self.release_on_bus(&request.team, &tasks, &mut removal).await;
        self.delete_local(&plan, &mut removal).await;
        tracing::info!(
            event = agent_core::log::LogEvent::TeamRemoved.as_str(),
            reason,
            bundle = %removal.bundle.as_ref().map_or_else(|| "none".to_owned(), |path| path.as_path().display().to_string()),
            freed_bytes = removal.freed_bytes,
            team = %request.team,
            tasks = tasks.len(),
            bundled = plan.bundled.len(),
            left_behind = removal.left_behind.len(),
            "team removed"
        );
        self.removal_notice(&removal, reason).await;
        Ok(Ok(removal))
    }

    /// Refuses active work unless forced, then ends and joins it before removal can touch its files.
    async fn settle_removal(
        &mut self,
        client: &Client,
        request: &RemoveRequest,
        tasks: &[TaskId],
        running: Vec<TaskId>,
    ) -> Result<Result<Vec<TaskId>, Failure>, OwnerError> {
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
            for task in tasks {
                self.stop_gates(task);
            }
            if tasks
                .iter()
                .any(|task| self.in_flight.contains(task) || self.parked.contains_key(task))
                && let Err(failure) = self
                    .control_request(client, Steer::Stop, &Target::Team(request.team.clone()))
                    .await?
            {
                return Ok(Err(failure));
            }
            while self.gate_answer_tasks.values().any(|task| tasks.contains(task)) {
                if let Some(joined) = self.gate_answers.join_next_with_id().await {
                    self.gate_answer_done(joined)?;
                }
            }
            while self
                .request_checking
                .values()
                .any(|checking| tasks.contains(checking.task()))
            {
                if let Some(joined) = self.request_checks.join_next_with_id().await {
                    self.request_check_done(joined).await?;
                }
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
        }
        let checks: Vec<_> = self.aborted_checks.iter().copied().collect();
        while checks.iter().any(|id| self.aborted_checks.contains(id)) {
            if let Some(joined) = self.checks.join_next_with_id().await {
                self.check_batch_done(joined).await?;
            }
        }
        Ok(Ok(running))
    }

    /// Reports a completed removal; notification failure cannot undo deleted files and is logged.
    async fn removal_notice(&mut self, removal: &Removal, reason: &str) {
        let rendered = (|| -> Result<String, Failure> {
            let bundle = removal.bundle.as_ref().map_or_else(
                || "none (all commits are in the base branch)".to_owned(),
                |path| path.as_path().display().to_string(),
            );
            let values = Variables::new()
                .with("team", removal.team.to_string())
                .and_then(|v| v.with("reason", reason))
                .and_then(|v| v.with("bundle", bundle))
                .and_then(|v| v.with("bytes", removal.freed_bytes.to_string()))
                .map_err(|error| internal("cannot build removal notice variables", &error))?;
            self.parts
                .texts
                .render(HarnessPrompt::NoticeTeamRemoved, &values)
                .map_err(|error| internal("cannot render removal notice", &error))
        })();
        match rendered {
            Ok(text) => {
                let (title, body) = text.split_once('\n').unwrap_or((text.as_str(), ""));
                self.deliver(title, body, "team removal notice").await;
            }
            Err(error) => tracing::warn!(
                event = agent_core::log::LogEvent::NotificationFailed.as_str(),
                team = %removal.team,
                error = %error.message,
                "cannot render the team removal notice"
            ),
        }
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
        let Measure { worktrees, task_dirs } = self.measure(team, &repo).await?;
        let mut branches = branches_of(team)?;
        if worktrees.is_empty() {
            let names: Vec<&str> = branches.iter().map(BranchName::as_str).collect();
            return Err(Failure {
                kind: FailureKind::Refused,
                message: format!(
                    "cannot remove team {team_id}: no worktree survives to reach its local branches ({}); \
                     keep one worktree, or bundle and delete the branches by hand; nothing was deleted",
                    names.join(", ")
                ),
            });
        }
        let mut existing = Vec::new();
        let mut bundled = Vec::new();
        let mut dirty = Vec::new();
        let mut heads = Vec::new();
        let (checkout, notes) = self.removal_checkout(&worktrees, &agent).await?;
        if let Some(checkout) = &checkout {
            let git = self.git_at(checkout, &agent);
            // A branch of the team is one whose name carries one of its task IDs, also a branch the state no longer names
            // (a split part's, say): it is bundled and deleted like the others.
            let listed = git
                .local_branches()
                .await
                .map_err(|error| internal("cannot list the branches", &error))?;
            for branch in listed {
                if tasks_of(team)
                    .iter()
                    .any(|task| branch.as_str().contains(task.as_str()))
                {
                    branches.insert(branch);
                }
            }
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
            let (inspected_dirty, inspected_heads) = inspect_worktrees(self, &worktrees, &git, &base, &agent).await?;
            dirty = inspected_dirty;
            heads = inspected_heads;
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
            heads,
            notes,
        })
    }

    /// Finds a repository through a surviving worktree, refusing damaged Git metadata.
    async fn removal_checkout(
        &self,
        worktrees: &[(TaskId, AbsolutePath, u64)],
        agent: &AgentName,
    ) -> Result<(Option<AbsolutePath>, Vec<String>), Failure> {
        // The checkout is the one of the first worktree that git can read; a directory that is not a checkout holds no
        // commits, and it is named in the reply rather than failing the removal.
        let mut checkout = None;
        let mut notes = Vec::new();
        for (task, dir, _) in worktrees {
            match checkout_of(&self.git_at(dir, agent)).await {
                Ok(found) => {
                    checkout = Some(found);
                    break;
                }
                Err(failure) => {
                    // A broken Git directory may still own unbundled commits: refuse before deleting anything.
                    if exists(&dir.as_path().join(".git")).await? {
                        return Err(failure);
                    }
                    notes.push(format!("worktree of {task} is not a git checkout: {}", failure.message));
                }
            }
        }
        Ok((checkout, notes))
    }

    /// The worktrees of `team` that exist and its task directories that exist, each with its bytes. The sizes are read
    /// on the blocking pool: a tree of files is walked there, not on the owner's loop.
    async fn measure(&self, team: &Team, repo: &RepoName) -> Result<Measure, Failure> {
        let home = self.parts.store.home();
        let mut worktrees = Vec::new();
        let mut task_dirs = Vec::new();
        for task in tasks_of(team) {
            let dir = home.worktree_dir(repo, &task);
            if exists(&dir).await? {
                let bytes = size_of(dir.clone()).await?;
                worktrees.push((task.clone(), absolute(&dir)?, bytes));
            }
            let dir = home.task_dir(&task);
            if exists(&dir).await? {
                let bytes = size_of(dir.clone()).await?;
                task_dirs.push((task, dir, bytes));
            }
        }
        Ok(Measure { worktrees, task_dirs })
    }

    /// Git for `dir`, with the identity of `agent`.
    pub(super) fn git_at(&self, dir: &AbsolutePath, agent: &AgentName) -> Git {
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
        if plan.bundled.is_empty() && plan.heads.is_empty() {
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
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| internal("cannot make the bundles directory", &error))?;
        }
        let bundle = absolute(&file)?;
        let git = self.git_at(checkout, &plan.agent);
        let mut refs: Vec<String> = plan
            .bundled
            .iter()
            .map(|branch| format!("refs/heads/{}", branch.as_str()))
            .collect();
        for (task, head) in &plan.heads {
            let name = head_ref(task);
            git.set_ref(&name, head)
                .await
                .map_err(|error| internal("cannot keep the HEAD of a worktree", &error))?;
            refs.push(name);
        }
        git.bundle_refs(&bundle, &refs)
            .await
            .map_err(|error| internal("cannot write the bundle of the team", &error))?;
        git.bundle_verify(&bundle)
            .await
            .map_err(|error| internal("the bundle of the team does not verify, nothing was removed", &error))?;
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

    /// Marks the team done on the bus and names what the bus keeps of its tasks, which the harness cannot release: the
    /// reservations stay until they expire or someone releases them, and the wake-ups of the bus are not removed.
    async fn release_on_bus(&mut self, team: &TeamId, tasks: &[TaskId], removal: &mut Removal) {
        removal.left_behind.push(
            "the bus itself was neither read for wake-ups nor cleaned; unread messages and reservations remain"
                .to_owned(),
        );
        match self.parts.bus.inv_open().await {
            Ok(requests) => {
                for request in requests {
                    if tasks.contains(&request.from_task)
                        && let Err(error) = self
                            .fail_investigation(&request, "the requester's task ended", None)
                            .await
                    {
                        removal.left_behind.push(format!(
                            "investigation {} could not be closed: {}",
                            request.id,
                            error_chain(&error)
                        ));
                    }
                }
            }
            Err(error) => removal.left_behind.push(format!(
                "open investigations on the bus cannot be read: {}",
                error_chain(&error)
            )),
        }
        if let Err(error) = self.parts.bus.team_done(team).await {
            removal.left_behind.push(format!(
                "the team is not marked done on the bus: {}",
                error_chain(&error)
            ));
        }
        match self.parts.bus.status().await {
            Ok(status) => {
                for line in status.reservations {
                    if tasks.iter().any(|task| line.contains(task.as_str())) {
                        removal.left_behind.push(format!(
                            "reservation on the bus, kept until it expires or is released: {line}"
                        ));
                    }
                }
            }
            Err(error) => removal.left_behind.push(format!(
                "the reservations of the team on the bus cannot be read: {}",
                error_chain(&error)
            )),
        }
    }

    /// Forgets what the owner keeps for the team and its tasks: their wake-ups, their holds, their queued messages.
    fn forget_team(&mut self, team: &TeamId, tasks: &[TaskId]) {
        self.looks.remove(team);
        self.closed.remove(team);
        self.outbox.retain(|outbound| outbound.team != *team);
        let gone = |task: &TaskId| tasks.contains(task);
        self.wakes.retain(|task, _| !gone(task));
        self.human_pauses.retain(|task| !gone(task));
        self.hangs.retain(|task, _| !gone(task));
        self.reasons.retain(|task, _| !gone(task));
        self.leads_being_told.retain(|task| !gone(task));
        self.lead_marks.retain(|task, _| !gone(task));
        self.wake_counts.retain(|task, _| !gone(task));
        self.wake_notes.retain(|task, _| !gone(task));
        self.serving.retain(|task, _| !gone(task));
        self.agent_waits.retain(|task| !gone(task));
        self.account_waits.retain(|task| !gone(task));
        self.halts.retain(|task, _| !gone(task));
        self.holds.retain(|task, _| !gone(task));
        self.incomplete.retain(|task, _| !gone(task));
        self.watched.retain(|task| !gone(task));
        self.cancels.retain(|task, _| !gone(task));
        self.gate_stops.retain(|task, _| !gone(task));
        self.task_plans.retain(|task, _| !gone(task));
        self.deferred_targets.retain(|task, _| !gone(task));
    }

    /// Deletes the worktrees, the task directories and the local branches of a removed team. A step that fails is
    /// named in `removal.left_behind`; nothing is ever deleted before its commits are in the bundle or on GitHub.
    async fn delete_local(&self, plan: &Plan, removal: &mut Removal) {
        for (task, dir, bytes) in &plan.worktrees {
            let dir_path = dir.as_path().to_path_buf();
            // A directory without a `.git` entry is not a worktree of the repository (a document or art work kept
            // in the same place): it has no commits and nothing git would keep, so it is removed as a tree.
            let is_worktree = match exists(&dir_path.join(".git")).await {
                Ok(found) => found,
                Err(failure) => {
                    removal
                        .left_behind
                        .push(format!("worktree of {task}: {}", failure.message));
                    continue;
                }
            };
            let outcome = match (is_worktree, &plan.checkout) {
                (false, _) => remove_tree(dir_path).await.map_err(|error| error_chain(&error)),
                (true, Some(checkout)) => self
                    .git_at(checkout, &plan.agent)
                    .worktree_remove(dir, RemoveMode::Force)
                    .await
                    .map_err(|error| error_chain(&error)),
                (true, None) => Err("no repository checkout is available to remove this Git worktree".to_owned()),
            };
            match outcome {
                Ok(()) => removal.freed_bytes = removal.freed_bytes.saturating_add(*bytes),
                Err(cause) => removal
                    .left_behind
                    .push(format!("worktree of {task} at {}: {cause}", dir.as_path().display())),
            }
        }
        if let Some(checkout) = &plan.checkout {
            let git = self.git_at(checkout, &plan.agent);
            for (task, _) in &plan.heads {
                if let Err(error) = git.delete_ref(&head_ref(task)).await {
                    removal
                        .left_behind
                        .push(format!("kept HEAD of {task}: {}", error_chain(&error)));
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
        for (task, dir, bytes) in &plan.task_dirs {
            match remove_tree(dir.clone()).await {
                Ok(()) => removal.freed_bytes = removal.freed_bytes.saturating_add(*bytes),
                Err(error) => removal.left_behind.push(format!(
                    "task directory of {task} at {}: {}",
                    dir.display(),
                    error_chain(&error)
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests;
