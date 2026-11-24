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
use crate::worker_request::{Pending, PendingError, write_pending};

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
    /// A worker task would reuse the requester's worktree.
    #[error("worker workspace cannot reuse requester task {task}")]
    AliasedTask {
        /// The aliased task.
        task: TaskId,
    },
    /// A saved branch does not belong to the dedicated worker task.
    #[error("workspace branch {found} does not match {expected}")]
    UnexpectedBranch {
        /// Branch derived from the worker and task.
        expected: BranchName,
        /// Untrusted branch in the record.
        found: BranchName,
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

/// Why a request's workspace cannot be completed: the worker's run ended and the result is to be settled in the
/// workspace.
#[derive(Debug, thiserror::Error)]
pub enum CompleteError {
    /// The pending of the result cannot be written into the worker's task directory.
    #[error("the pending of the result of {request} cannot be written into the worker's task directory")]
    Pending {
        /// The request.
        request: RequestId,
        /// The failure.
        source: PendingError,
    },
    /// The record cannot be saved at [`Stage::Ended`].
    #[error("the record of the workspace of {request} cannot be saved as ended")]
    Workspace {
        /// The request.
        request: RequestId,
        /// The failure.
        source: WorkspaceError,
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
/// The worktree is at [`Home::worktree_dir`]`(requester_repo, worker_task)` on the branch derived from its worker and task; the record
/// is kept in the requester's task directory (`request-workspace-<number>.json`), so that lookups of completion,
/// pending settlement and second try find it without the requester's crew. The workspace is released — its worktree and
/// branch removed — when the request's result is settled; that is the retention policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecord {
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

/// A workspace whose worker task and branch cannot alias the requester.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "WorkspaceRecord")]
pub struct RequestWorkspace {
    /// Validated metadata; callers can only borrow it immutably.
    #[serde(flatten)]
    record: WorkspaceRecord,
}

impl RequestWorkspace {
    /// Borrows the workspace metadata without permitting changes to its identity.
    #[must_use]
    pub fn record(&self) -> &WorkspaceRecord {
        &self.record
    }
}

impl TryFrom<WorkspaceRecord> for RequestWorkspace {
    type Error = WorkspaceError;

    /// Seals a dedicated worker task and its derived branch.
    ///
    /// # Errors
    /// Returns [`WorkspaceError::AliasedTask`] for the requester's task,
    /// [`WorkspaceError::UnexpectedBranch`] for another branch, or
    /// [`WorkspaceError::Branch`] if the derived branch is invalid.
    fn try_from(record: WorkspaceRecord) -> Result<Self, Self::Error> {
        if record.worker_task == record.requester_task {
            return Err(WorkspaceError::AliasedTask {
                task: record.worker_task,
            });
        }
        let expected = branch(&record.worker, &record.worker_task)?;
        if record.branch != expected {
            return Err(WorkspaceError::UnexpectedBranch {
                expected,
                found: record.branch,
            });
        }
        Ok(Self { record })
    }
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
    let path = path_of(task_dir, workspace.record.request);
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
        request: workspace.record.request,
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| WorkspaceError::Write {
        request: workspace.record.request,
        source,
    })
}

/// Completes a request whose worker's run ended in its workspace: the pending of the result is written into the worker's
/// task directory first, then the record is saved at [`Stage::Ended`] in the requester's task directory. A daemon that
/// stops between the two writes leaves a `Working` record whose pending is already present; the settlement pass of the
/// owner completes it and settles it, so a stop never loses the result or forgets the workspace.
///
/// # Errors
/// [`CompleteError::Pending`] when the pending cannot be written; [`CompleteError::Workspace`] when the record cannot be
/// saved at `Ended`.
pub fn complete(home: &Home, workspace: &mut RequestWorkspace, pending: &Pending) -> Result<(), CompleteError> {
    let worker_dir = home.task_dir(&workspace.record.worker_task);
    std::fs::create_dir_all(&worker_dir).map_err(|source| CompleteError::Pending {
        request: workspace.record.request,
        source: PendingError::Write {
            path: worker_dir.clone(),
            source,
        },
    })?;
    write_pending(&worker_dir, pending).map_err(|source| CompleteError::Pending {
        request: workspace.record.request,
        source,
    })?;
    workspace.record.stage = Stage::Ended;
    save(&home.task_dir(&workspace.record.requester_task), workspace).map_err(|source| CompleteError::Workspace {
        request: workspace.record.request,
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
    if request_of_file(path) == Some(workspace.record.request) {
        Ok(workspace)
    } else {
        Err(WorkspaceError::Mismatch {
            path: path.to_owned(),
            found: workspace.record.request.number(),
        })
    }
}

/// Creates the workspace of `request` in the `home`: a worktree of the requester's `git` at `base`, on
/// the branch derived from its worker and task, and records it in the requester's task directory.
///
/// The requester's uncommitted files stay in the requester's own worktree untouched: the workspace's worktree is added
/// for the worker's task, from the requester's committed `base`.
///
/// # Errors
/// [`WorkspaceError`] when the worktree cannot be added, or the record cannot be saved.
pub async fn prepare(home: &Home, git: &Git, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    let worktree = worktree_of(home, &workspace.record.requester_repo, &workspace.record.worker_task)?;
    git.worktree_add(
        &worktree,
        &WorktreeBranch::New {
            branch: workspace.record.branch.clone(),
            start: Revision::Commit(workspace.record.base.clone()),
        },
    )
    .await
    .map_err(|source| WorkspaceError::Git {
        request: workspace.record.request,
        source,
    })?;
    let task_dir = home.task_dir(&workspace.record.requester_task);
    let record = write_record(&task_dir, workspace).await;
    if let Err(write_error) = record {
        // A failed record write rolls the new worktree and its branch back, so a failed prepare leaves nothing
        // behind; a rollback that fails too is told as a warning, and the leftover worktree stays discoverable by
        // the workspace's branch name.
        if let Err(rollback_error) = remove_worktree_and_branch(home, git, workspace).await {
            tracing::warn!(
                event = agent_core::log::LogEvent::GitCallFailed.as_str(),
                request = %workspace.record.request,
                error = %rollback_error,
                "the worktree of a failed prepare could not be rolled back"
            );
        }
        return Err(write_error);
    }
    Ok(())
}

/// Releases the workspace of `request` in the `home`: removes its worktree and its branch, and the record of it kept in
/// the requester's task directory.
///
/// A release is retriable: a worktree, branch or record that a previous partial release already removed is not a
/// failure again.
///
/// # Errors
/// [`WorkspaceError`] when a worktree or branch cannot be removed, or the record cannot be removed.
pub async fn release(home: &Home, git: &Git, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    remove_worktree_and_branch(home, git, workspace).await?;
    let path = path_of(
        &home.task_dir(&workspace.record.requester_task),
        workspace.record.request,
    );
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(WorkspaceError::Write {
            request: workspace.record.request,
            source,
        }),
    }
}

/// Writes the record of `workspace` into the requester's task directory `task_dir`, through a temporary file.
///
/// # Errors
/// [`WorkspaceError::Write`] when the directory or the file cannot be written; [`WorkspaceError::Encode`] when the
/// workspace cannot be made JSON.
async fn write_record(task_dir: &Path, workspace: &RequestWorkspace) -> Result<(), WorkspaceError> {
    tokio::fs::create_dir_all(task_dir)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.record.request,
            source,
        })?;
    let (path, text) = file_and_text(task_dir, workspace)?;
    let temporary = path.with_extension("json.tmp");
    tokio::fs::write(&temporary, text)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.record.request,
            source,
        })?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|source| WorkspaceError::Write {
            request: workspace.record.request,
            source,
        })
}

/// Removes the worktree and the branch of `workspace` from the `home` when they are still there.
///
/// The retriable removal of [`release`] and the rollback of a failed [`prepare`] share it: a worktree is only removed
/// when its branch is still checked out somewhere, and the branch only when it still exists.
///
/// # Errors
/// [`WorkspaceError::Git`] when a worktree or branch that is still there cannot be removed.
async fn remove_worktree_and_branch(
    home: &Home,
    git: &Git,
    workspace: &RequestWorkspace,
) -> Result<(), WorkspaceError> {
    let worktree = worktree_of(home, &workspace.record.requester_repo, &workspace.record.worker_task)?;
    let held = git.held_branches().await.map_err(|source| WorkspaceError::Git {
        request: workspace.record.request,
        source,
    })?;
    if held.contains(&workspace.record.branch) {
        git.worktree_remove(&worktree, RemoveMode::Force)
            .await
            .map_err(|source| WorkspaceError::Git {
                request: workspace.record.request,
                source,
            })?;
    }
    if git
        .branch_tip(&workspace.record.branch)
        .await
        .map_err(|source| WorkspaceError::Git {
            request: workspace.record.request,
            source,
        })?
        .is_some()
    {
        git.branch_delete(&workspace.record.branch, DeleteMode::Force)
            .await
            .map_err(|source| WorkspaceError::Git {
                request: workspace.record.request,
                source,
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
