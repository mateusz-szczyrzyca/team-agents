//! The record of a request's workspace: the task and worktree a worker serves one request in, made in the requester's
//! repository at the requester's commit and kept with no place in the requester's crew.
//!
//! From part c of the item on, a request of a worker of the pool is served in a workspace of its own: a task and a git
//! worktree of that task, in the requester's repository, created at the requester's committed end, so that the
//! requester's uncommitted files stay in their own worktree untouched. The workspace is kept from the moment the
//! request is given to the worker until its result is settled, then released — its worktree and branch removed — by
//! the retention policy told in `docs/architecture.md` ("Request workspaces").
//!
//! This module is the persisted record and the helpers that create and release such a workspace, so that completion,
//! pending-settlement and second-try lookups can find a request's worker task and worktree without going through the
//! requester's crew. The record is kept in the requester's task directory, which the request names; finding it never
//! needs the crew.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue, RepoName};
use agent_core::ids::{AgentName, BranchName, InvalidBranchName, RoundId, TaskId};
use agent_core::state::CommitId;
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;
use crate::git::{DeleteMode, Git, GitError, RemoveMode, Revision, WorktreeBranch};
use crate::store::Home;

/// The start of the name of the file that keeps a request's workspace: `request-workspace-<number>.json` in the
/// requester's task directory.
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
    /// A branch of the workspace cannot be read from its text.
    #[error("the branch of the workspace of {worker} in {task} cannot be read from its text")]
    Branch {
        /// The worker.
        worker: AgentName,
        /// The worker's task.
        task: TaskId,
        /// The failure.
        source: InvalidBranchName,
    },
    /// The worktree of the workspace cannot be read from its path: the path is not UTF-8 text.
    #[error("the worktree of {task} in {repo} is not UTF-8 text at {path:?}")]
    NotText {
        /// The repository.
        repo: RepoName,
        /// The task.
        task: TaskId,
        /// The path.
        path: PathBuf,
    },
    /// The worktree of the workspace cannot be read from its path: it is not an absolute path.
    #[error("the worktree of {task} in {repo} is not an absolute path")]
    NotAbsolute {
        /// The repository.
        repo: RepoName,
        /// The task.
        task: TaskId,
        /// The failure.
        source: InvalidValue,
    },
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
///
/// # Errors
/// [`WorkspaceError::Branch`] when the names make no branch.
pub fn branch(worker: &AgentName, task: &TaskId) -> Result<BranchName, WorkspaceError> {
    format!("agents/{worker}/{task}")
        .parse()
        .map_err(|source| WorkspaceError::Branch {
            worker: worker.clone(),
            task: task.clone(),
            source,
        })
}

/// The absolute path of the worktree of the worker task `task` of the requester's repository `repo` in the `home`.
///
/// # Errors
/// [`WorkspaceError::NotText`] when the path is not UTF-8 text; [`WorkspaceError::NotAbsolute`] when it is not absolute.
pub fn worktree_of(home: &Home, repo: &RepoName, task: &TaskId) -> Result<AbsolutePath, WorkspaceError> {
    let path = home.worktree_dir(repo, task);
    let text = path.to_str().ok_or_else(|| WorkspaceError::NotText {
        repo: repo.clone(),
        task: task.clone(),
        path: path.clone(),
    })?;
    AbsolutePath::from_str(text).map_err(|source| WorkspaceError::NotAbsolute {
        repo: repo.clone(),
        task: task.clone(),
        source,
    })
}

/// A request's workspace: the worker, its task and worktree, in the requester's repository at the requester's commit.
///
/// The worktree is at [`Home::worktree_dir`]`(requester_repo, worker_task)` on [`RequestWorkspace::branch`]; the record
/// is kept in the requester's task directory (`request-workspace-<number>.json`), so that lookups of completion,
/// pending settlement and second try find it without the requester's crew. The workspace is released — its worktree and
/// branch removed — when the request's result is settled; that is the retention policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestWorkspace {
    /// The request.
    pub request: RequestId,
    /// The requester.
    pub requester: AgentName,
    /// The requester's task.
    pub requester_task: TaskId,
    /// The round the requester's last run worked for, under which a snapshot of its worktree is kept.
    pub requester_round: RoundId,
    /// The repository the requester works in; the workspace's worktree is of this repository.
    pub requester_repo: RepoName,
    /// The worker that serves the request.
    pub worker: AgentName,
    /// The worker's task; the workspace's worktree and task directory are of this task.
    pub worker_task: TaskId,
    /// The branch of the workspace's worktree.
    pub branch: BranchName,
    /// The commit the requester's worktree was at when it was committed; the workspace's worktree starts here.
    pub base: CommitId,
    /// How far its request's result got; each stage is kept as the harness settles it.
    pub stage: Stage,
}

/// How far the settlement of a request's result has got; written before each next stage.
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

/// The request whose workspace's file is `path`, from its name alone; `None` when the name is not of the form.
#[must_use]
pub fn request_of_file(path: &Path) -> Option<RequestId> {
    let digits = path
        .file_name()?
        .to_str()?
        .strip_prefix(FILE_PREFIX)?
        .strip_suffix(".json")?;
    digits.parse::<std::num::NonZeroU64>().ok().map(RequestId::new)
}

/// The file of `workspace` in the requester's task directory `task_dir` and the text written to it.
///
/// # Errors
/// [`WorkspaceError::Encode`] when the workspace cannot be made JSON.
fn file_and_text(task_dir: &Path, workspace: &RequestWorkspace) -> Result<(PathBuf, String), WorkspaceError> {
    let path = path_of(task_dir, workspace.request);
    let text = serde_json::to_string_pretty(workspace).map_err(WorkspaceError::Encode)?;
    Ok((path, text))
}

/// Writes `workspace` to its file in the requester's task directory `task_dir`, through a temporary file, so that a
/// reader never sees half of it. A blocking write is the simplest that is also atomic; the async [`prepare`] writes
/// the record of a new workspace with `tokio::fs` the same way.
///
/// # Errors
/// [`WorkspaceError::Write`] when the file cannot be written; [`WorkspaceError::Encode`] when it cannot be made JSON.
pub fn save(task_dir: &Path, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let (path, text) = file_and_text(task_dir, workspace)?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|source| WorkspaceError::Write {
        request: workspace.request,
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| WorkspaceError::Write {
        request: workspace.request,
        source,
    })
}

/// The workspace files kept in the requester's task directory `task_dir`, each read on its own, oldest first by number:
/// a file that cannot be read or is not a workspace is an error of its own and does not hide the others. A directory
/// that is not there holds none.
///
/// # Errors
/// [`WorkspaceError::List`] when the directory cannot be listed.
pub async fn read_each(task_dir: &Path) -> Result<Vec<Result<RequestWorkspace, WorkspaceError>>, WorkspaceError> {
    let mut entries = match tokio::fs::read_dir(task_dir).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(WorkspaceError::List {
                path: task_dir.to_owned(),
                source,
            });
        }
    };
    let mut paths = Vec::new();
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(source) => {
                return Err(WorkspaceError::List {
                    path: task_dir.to_owned(),
                    source,
                });
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if is_workspace_name(name) {
            paths.push(entry.path());
        }
    }
    let number = |path: &Path| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix(FILE_PREFIX))
            .and_then(|digits| digits.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    };
    paths.sort_by_key(|path| number(path));
    let mut read = Vec::new();
    for path in paths {
        read.push(read_one(&path).await);
    }
    Ok(read)
}

/// Reads the workspace of one file, checking that it is the request its name says.
async fn read_one(path: &Path) -> Result<RequestWorkspace, WorkspaceError> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|source| WorkspaceError::Read {
            path: path.to_owned(),
            source,
        })?;
    let workspace = serde_json::from_str::<RequestWorkspace>(&text).map_err(|source| WorkspaceError::Parse {
        path: path.to_owned(),
        source,
    })?;
    if request_of_file(path) == Some(workspace.request) {
        Ok(workspace)
    } else {
        Err(WorkspaceError::Mismatch {
            path: path.to_owned(),
            found: workspace.request.number(),
        })
    }
}

/// Creates the workspace of `request` in the `home`: a worktree of the requester's `git` at `base`, on
/// [`RequestWorkspace::branch`], and records it in the requester's task directory.
///
/// The requester's uncommitted files stay in the requester's own worktree untouched: the workspace's worktree is added
/// for the worker's task, from the requester's committed `base`.
///
/// # Errors
/// [`WorkspaceError`] when the worktree cannot be added, or the record cannot be saved.
pub async fn prepare(home: &Home, git: &Git, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let worktree = worktree_of(home, &workspace.requester_repo, &workspace.worker_task)?;
    git.worktree_add(
        &worktree,
        &WorktreeBranch::New {
            branch: workspace.branch.clone(),
            start: Revision::Commit(workspace.base.clone()),
        },
    )
    .await
    .map_err(|source| WorkspaceError::Git {
        request: workspace.request,
        source,
    })?;
    let task_dir = home.task_dir(&workspace.requester_task);
    tokio::fs::create_dir_all(&task_dir)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.request,
            source,
        })?;
    let (path, text) = file_and_text(&task_dir, workspace)?;
    let temporary = path.with_extension("json.tmp");
    tokio::fs::write(&temporary, text)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.request,
            source,
        })?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.request,
            source,
        })
}

/// Releases the workspace of `request` in the `home`: removes its worktree and its branch, and the record of it kept in
/// the requester's task directory.
///
/// # Errors
/// [`WorkspaceError`] when a worktree or branch cannot be removed, or the record cannot be removed.
pub async fn release(home: &Home, git: &Git, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let worktree = worktree_of(home, &workspace.requester_repo, &workspace.worker_task)?;
    git.worktree_remove(&worktree, RemoveMode::Force)
        .await
        .map_err(|source| WorkspaceError::Git {
            request: workspace.request,
            source,
        })?;
    git.branch_delete(&workspace.branch, DeleteMode::Force)
        .await
        .map_err(|source| WorkspaceError::Git {
            request: workspace.request,
            source,
        })?;
    let path = path_of(&home.task_dir(&workspace.requester_task), workspace.request);
    tokio::fs::remove_file(path)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.request,
            source,
        })?;
    Ok(())
}

#[cfg(test)]
mod tests;
