//! Best-effort worktree cleanup after all the team's rounds have merged.

use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::{AgentName, TaskId, TeamId};
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
    /// Releases completed workspaces only after every item has merged and the next round cannot need them.
    /// Cleanup is a side effect: failure is logged and never reverses a durable merge or prevents team closure.
    pub(in crate::daemon::owner) async fn cleanup_merged_worktrees(&self, team_id: &TeamId) {
        let Some(team) = self.parts.state.team(team_id) else {
            return;
        };
        if team.items().is_empty()
            || team.open_rounds().next().is_some()
            || !team.items().iter().all(|item| {
                matches!(
                    item.phase(),
                    ItemPhase::Done {
                        completion: Completion::Merged { .. }
                    }
                )
            })
        {
            return;
        }
        let members = team
            .crew()
            .roster()
            .into_iter()
            .map(|(_, member)| member)
            .chain(team.reviewers());
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
                Ok(Cleanup::Dirty) => tracing::warn!(event = LogEvent::GitCallFailed.as_str(),
                    team = %team_id, task = %member.task(),
                    "the merged worktree is kept: uncommitted changes"),
                Ok(Cleanup::Unpushed) => tracing::warn!(event = LogEvent::GitCallFailed.as_str(),
                    team = %team_id, task = %member.task(),
                    "the merged worktree is kept: commits not on origin"),
                Err(error) => tracing::warn!(event = LogEvent::GitCallFailed.as_str(),
                    team = %team_id, task = %member.task(), error = %error_chain(&error),
                    "the merged worktree cannot be removed and is kept"),
            }
        }
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
