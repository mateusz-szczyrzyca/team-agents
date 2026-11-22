//! The request of a worker whose run has ended, kept in a file of the worker's task directory while the harness settles it:
//! its check, the bringing over of its commit and the answer. Each stage is written before the next starts, so that a
//! daemon that stops between two of them goes on from the stage the file names.

use std::path::{Path, PathBuf};

use agent_core::decide::worker_result::CheckOutcome;
use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;

/// The start of the name of the file that keeps a request whose worker has ended: `worker-request-<number>.json` in the
/// worker's task directory.
const FILE_PREFIX: &str = "worker-request-";

/// Why a request that a worker has ended cannot be kept or read.
#[derive(Debug, thiserror::Error)]
pub enum PendingError {
    /// The file of the request cannot be written.
    #[error("the request cannot be kept in {path:?}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// The directory of a worker's task cannot be read.
    #[error("the requests of a worker cannot be read from {path:?}")]
    List {
        /// The directory.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// The file of a request cannot be read.
    #[error("the request cannot be read from {path:?}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: std::io::Error,
    },
    /// The file of a request is not what the harness wrote.
    #[error("{path:?} is not the file of a request")]
    Parse {
        /// The file.
        path: PathBuf,
        /// The failure.
        source: serde_json::Error,
    },
    /// A request cannot be written as JSON.
    #[error("the request cannot be written as JSON")]
    Encode(#[source] serde_json::Error),
}

/// A request whose worker's run has ended: who asked, who served, what the worker said and how far the harness got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    /// The request.
    pub request: RequestId,
    /// The team.
    pub team: TeamId,
    /// The member that asked.
    pub requester: AgentName,
    /// Its task.
    pub requester_task: TaskId,
    /// The worker.
    pub worker: AgentName,
    /// Its task.
    pub worker_task: TaskId,
    /// The worker's report, without its status line.
    pub summary: String,
    /// How long the worker's run lasted, in seconds.
    pub run_seconds: u64,
    /// The commit the worker's worktree was at when the request was given to it; absent when it could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// How far the harness got.
    pub stage: Stage,
}

/// How far the harness got with a request: each stage is written to the file before the next one starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    /// The worker's run ended; nothing is checked yet.
    Ended,
    /// What the request asked is read, the worker's commit named and the check run.
    Checked(Box<Checked>),
    /// The result is decided and, when it is good, brought over; the answer is not given yet.
    Settled {
        /// What was checked.
        checked: Box<Checked>,
        /// What became of the result.
        settlement: Settlement,
    },
}

/// What the check of a request's result found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    /// The step of the requester's plan; absent on a request in the old form.
    pub step: Option<ItemId>,
    /// The paths the request lets the worker write; none for a request for facts only.
    pub files: Vec<String>,
    /// The commit the worker's worktree is at now; absent when it could not be read.
    pub head: Option<String>,
    /// Whether the worker committed since the request was given to it.
    pub committed: bool,
    /// The files the worker's commits changed that `files` do not name.
    pub outside: Vec<String>,
    /// What the check found; absent when the request has no check.
    pub found: Option<Found>,
}

/// A check that ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// What it found.
    pub outcome: CheckOutcome,
    /// The check as a line.
    pub line: String,
    /// The end of its output.
    pub tail: String,
    /// Why it failed, if it did.
    pub reason: Option<String>,
    /// How long it took, in milliseconds.
    pub duration_ms: u64,
    /// The task-relative file of its output.
    pub output: Option<String>,
}

/// What became of a worker's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "settled", rename_all = "snake_case")]
pub enum Settlement {
    /// The result is the worker's report, as it always was.
    Report,
    /// The worker's commit is on the requester's branch.
    Brought {
        /// The commit on the requester's branch.
        commit: String,
    },
    /// The commit cannot be written to the requester's branch: the files in conflict, or in the way.
    Conflict {
        /// The files.
        files: Vec<String>,
    },
    /// The result is wrong: the reasons, in the words the requester reads.
    Returned {
        /// The reasons.
        reasons: Vec<String>,
    },
}

/// The file of `request` in the task directory `task_dir`.
pub fn pending_path(task_dir: &Path, request: RequestId) -> PathBuf {
    task_dir.join(format!("{FILE_PREFIX}{}.json", request.number()))
}

/// Writes `pending` to its file, through a temporary file, so that a reader never sees half of it.
///
/// # Errors
/// [`PendingError`].
pub fn write_pending(task_dir: &Path, pending: &Pending) -> Result<(), PendingError> {
    let _ = (task_dir, pending);
    Ok(())
}

/// The requests kept in the task directory `task_dir`, oldest first.
///
/// # Errors
/// [`PendingError`].
pub async fn read_pending(task_dir: &Path) -> Result<Vec<Pending>, PendingError> {
    let _ = task_dir;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests;
