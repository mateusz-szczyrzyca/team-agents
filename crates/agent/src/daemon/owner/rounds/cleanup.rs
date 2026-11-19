//! Best-effort worktree cleanup after all the team's rounds have merged.

use std::path::PathBuf;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::{AgentName, RoundId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::state::{Completion, ItemPhase};
use agent_core::status::TaskStatus;
use agent_runtime::git::{RemoveMode, TreeState};
use agent_runtime::process::error_chain;

use super::{Owner, PublishError, agent_name};

/// The outcome of checking and releasing one merged task's worktree.
#[derive(Debug)]
enum Cleanup {
    /// Git removed the checkout and retained its branch.
    Removed,
    /// Uncommitted work must remain available for inspection.
    Dirty,
    /// At least one commit has not reached a known branch on origin.
    Unpushed,
}

impl Owner {
    /// Removes the `target/` directories of all tasks of `round`, including those of kept worktrees. A task whose run is
    /// still going keeps its directory until the run ends, so a build in progress is not cut short.
    pub(in crate::daemon::owner) fn cleanup_round_targets(&mut self, team_id: &TeamId, round: &RoundId) {
        let Some(team) = self.parts.state.team(team_id) else {
            return;
        };
        let targets: Vec<(TaskId, PathBuf)> = team
            .members_in(round)
            .into_iter()
            .map(|(_, member)| {
                let dir = self.parts.store.home().worktree_dir(team.repo(), member.task());
                (member.task().clone(), dir)
            })
            .collect();
        for (task, worktree_dir) in targets {
            if self.in_flight.contains(&task) {
                self.deferred_targets.insert(task, worktree_dir);
            } else {
                self.remove_target(&task, &worktree_dir);
            }
        }
    }

    /// Releases completed workspaces only after every item is done, at least one merged, and no round is open.
    /// Cleanup is a side effect: failure is logged and never reverses a durable merge or prevents team closure.
    pub(in crate::daemon::owner) async fn cleanup_merged_worktrees(&mut self, team_id: &TeamId) {
        let Some(team) = self.parts.state.team(team_id) else {
            return;
        };
        if team.open_rounds().next().is_some() {
            return;
        }
        // The team ended by a merge: every item is done, and at least one by its pull request; the others the list
        // ticked before the team started.
        let mut merged = false;
        for item in team.items() {
            let ItemPhase::Done { completion } = item.phase() else {
                return;
            };
            match completion {
                Completion::Merged { .. } => merged = true,
                Completion::Listed => {}
            }
        }
        if !merged {
            return;
        }
        let members = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers());
        let mut kept = Vec::new();
        for member in members {
            if member.status() == TaskStatus::Running {
                continue;
            }
            let result = match agent_name(member.agent()) {
                Ok(agent) => self.cleanup_worktree(team.repo(), member.task(), &agent).await,
                Err(error) => {
                    tracing::warn!(event = LogEvent::GitCallFailed.as_str(), team = %team_id,
                        task = %member.task(), error = %error_chain(&error),
                        "the merged worktree is kept: its agent identity cannot be read");
                    continue;
                }
            };
            match result {
                Ok(Cleanup::Removed) => {}
                Ok(Cleanup::Dirty) => {
                    kept.push(self.worktree_target(team.repo(), member.task()));
                    tracing::warn!(event = LogEvent::GitCallFailed.as_str(),
                        team = %team_id, task = %member.task(),
                        "the merged worktree is kept: uncommitted changes");
                }
                Ok(Cleanup::Unpushed) => {
                    kept.push(self.worktree_target(team.repo(), member.task()));
                    tracing::warn!(event = LogEvent::GitCallFailed.as_str(),
                        team = %team_id, task = %member.task(),
                        "the merged worktree is kept: commits not on origin");
                }
                Err(error) => {
                    kept.push(self.worktree_target(team.repo(), member.task()));
                    tracing::warn!(event = LogEvent::GitCallFailed.as_str(),
                        team = %team_id, task = %member.task(), error = %error_chain(&error),
                        "the merged worktree cannot be removed and is kept");
                }
            }
        }
        for (task, worktree_dir) in kept {
            self.remove_target(&task, &worktree_dir);
        }
    }

    /// The task and the directory of its worktree, whose build directory is removed for a worktree kept at the end of
    /// its team: the round closed before its work was finished, so its directory may hold a build made since.
    fn worktree_target(&self, repo: &RepoName, task: &TaskId) -> (TaskId, PathBuf) {
        (task.clone(), self.parts.store.home().worktree_dir(repo, task))
    }

    /// Removes one clean, pushed worktree using Git's normal safety checks; its branches are never deleted.
    async fn cleanup_worktree(
        &self,
        repo: &RepoName,
        task: &TaskId,
        agent: &AgentName,
    ) -> Result<Cleanup, PublishError> {
        let git = self.worktree_git(repo, task, agent)?;
        if let TreeState::Dirty(_) = git.tree_state().await? {
            return Ok(Cleanup::Dirty);
        }
        git.refresh_origin_branches().await?;
        if !git.head_on_origin().await? {
            return Ok(Cleanup::Unpushed);
        }
        let path = self.parts.store.home().worktree_dir(repo, task);
        let text = path.to_str().ok_or_else(|| PublishError::Path {
            path: path.clone(),
            source: None,
        })?;
        let absolute = AbsolutePath::from_str(text).map_err(|source| PublishError::Path {
            path,
            source: Some(source),
        })?;
        git.worktree_remove(&absolute, RemoveMode::Clean).await?;
        Ok(Cleanup::Removed)
    }
}
