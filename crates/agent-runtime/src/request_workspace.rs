//! The record of a request's workspace: the task and worktree a worker serves one request in, made in the requester's
//! repository at the requester's commit and kept with no place in the requester's crew.
//!
//! This module is the persisted record and the helpers that create and release such a workspace, so that completion,
//! pending-settlement and second-try lookups can find a request's worker task and worktree without going through the
//! requester's crew.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::{AgentName, BranchName, RoundId, TaskId};
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;
use crate::git::{Git, GitError};
use crate::store::Home;

/// The start of the name of the file that keeps a request's workspace.
const FILE_PREFIX: &str = "request-workspace-";

/// Why a request's workspace cannot be recorded, read or released.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// The file of the workspace cannot be written.
    #[error("the workspace of {request} cannot be written")]
    Write {
        /// The request.
        request: RequestId,
        /// The failure.
        source: std::io::Error,
    },
    /// The workspace files of a task directory cannot be listed.
    #[error("the workspace files of {path:?} cannot be listed")]
    List {
        /// The directory.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// A workspace cannot be read from its file.
    #[error("the workspace at {path:?} cannot be read")]
    Read {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// A workspace's file is not what the harness wrote.
    #[error("{path:?} is not the record of a request's workspace")]
    Parse {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: serde_json::Error,
    },
    /// A workspace's file holds another request than the one its name says.
    #[error("{path:?} holds the workspace of request H{found}, not the one in its name")]
    Mismatch {
        /// The file.
        path: PathBuf,
        /// The number of the request it holds.
        found: std::num::NonZeroU64,
    },
    /// A need name cannot be read from its text.
    #[error("cannot read a name: {0:?}")]
    Name(String),
    /// A workspace cannot be written as JSON.
    #[error("the workspace cannot be written as JSON")]
    Encode(#[source] serde_json::Error),
    /// A git operation that made or released the workspace's worktree failed.
    #[error("git of the workspace of {request} failed")]
    Git {
        /// The request.
        request: RequestId,
        /// The failure.
        source: GitError,
    },
}

/// The branch of the workspace a worker serves a request in, in the requester's repository.
#[must_use]
pub fn branch(worker: &AgentName, task: &TaskId) -> Result<BranchName, WorkspaceError> {
    format!("agents/{worker}/{task}")
        .parse()
        .map_err(|_| WorkspaceError::Name("a branch of a request's workspace".to_owned()))
}

/// The absolute path of the worktree of the worker task `task` of the requester's repository `repo` in the `home`.
pub fn worktree_of(home: &Home, repo: &RepoName, task: &TaskId) -> Result<AbsolutePath, WorkspaceError> {
    let path = home.worktree_dir(repo, task);
    let text = path
        .to_str()
        .ok_or_else(|| WorkspaceError::Name(format!("the worktree of {task} in {repo}")))?;
    AbsolutePath::from_str(text).map_err(|source| WorkspaceError::Name(format!("the worktree of {task}: {source}")))
}

/// A request's workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestWorkspace {
    /// The request.
    pub request: RequestId,
    /// The requester.
    pub requester: AgentName,
    /// The requester's task.
    pub requester_task: TaskId,
    /// The round the requester's last run worked for.
    pub requester_round: RoundId,
    /// The repository the requester works in.
    pub requester_repo: RepoName,
    /// The worker that serves the request.
    pub worker: AgentName,
    /// The worker's task.
    pub worker_task: TaskId,
    /// The branch of the workspace's worktree.
    pub branch: BranchName,
    /// The commit the requester's worktree was at when it was committed.
    pub base: String,
    /// How far its request's result got.
    pub stage: Stage,
}

/// How far the settlement of a request's result has got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "workspace_stage", rename_all = "snake_case")]
pub enum Stage {
    /// The workspace is made and the worker works in it.
    Working,
    /// The worker's run ended and the request waits to be settled in this workspace.
    Ended,
}

/// The file that keeps the workspace of `request` in the requester's task directory `task_dir`.
#[must_use]
pub fn path_of(task_dir: &Path, request: RequestId) -> PathBuf {
    task_dir.join(format!("{FILE_PREFIX}{}.json", request.number()))
}

/// Whether the file name `name` is the file of a request's workspace.
fn is_workspace_name(name: &str) -> bool {
    name.starts_with(FILE_PREFIX) && Path::new(name).extension().is_some_and(|ext| ext == "json")
}

/// The request whose workspace's file is `path`, from its name alone.
#[must_use]
pub fn request_of_file(path: &Path) -> Option<RequestId> {
    let digits = path.file_name()?.to_str()?.strip_prefix(FILE_PREFIX)?.strip_suffix(".json")?;
    digits.parse::<std::num::NonZeroU64>().ok().map(RequestId::new)
}

/// Writes `workspace` to its file in the requester's task directory `task_dir`.
pub fn save(task_dir: &Path, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let _ = (task_dir, workspace);
    Err(WorkspaceError::Name("not implemented".to_owned()))
}

/// The workspace files kept in the requester's task directory `task_dir`, each read on its own.
pub async fn read_each(task_dir: &Path) -> Result<Vec<Result<RequestWorkspace, WorkspaceError>>, WorkspaceError> {
    let _ = task_dir;
    Ok(Vec::new())
}

/// Creates the workspace of `request` in the `home`.
pub async fn prepare(home: &Home, git: &Git, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let _ = (home, git, workspace);
    Err(WorkspaceError::Name("not implemented".to_owned()))
}

/// Releases the workspace of `request` in the `home`.
pub async fn release(home: &Home, git: &Git, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let _ = (home, git, workspace);
    Err(WorkspaceError::Name("not implemented".to_owned()))
}

#[cfg(test)]
mod tests;