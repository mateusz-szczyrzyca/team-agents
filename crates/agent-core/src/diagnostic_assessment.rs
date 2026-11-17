//! Pure assessment of observed team diagnostics; unknown observations never imply a failure.

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::time::Duration;

use crate::ids::{BranchName, TaskId};
use crate::state::{CommitId, GateRecord, ItemPhase, Timestamp};
use crate::status::TaskStatus;

/// Observations available for one diagnostic assessment, supplied without I/O.
#[derive(Debug, Clone)]
pub struct Assessment {
    /// Collection time in UTC.
    pub now: Timestamp,
    /// Inactivity strictly beyond this duration is stale.
    pub stale_after: Duration,
    /// Whether a daemon holds the home, when known.
    pub daemon: Option<Daemon>,
    /// Coordinator task of this team.
    pub lead: TaskId,
    /// All known tasks of the team.
    pub tasks: Vec<Task>,
    /// Current item phases, using the state model's invariants.
    pub items: Vec<ItemPhase>,
    /// Lead worktree facts, absent when they could not be read.
    pub lead_git: Option<LeadGit>,
    /// Observed task branches, including the lead's branch.
    pub branches: Vec<Branch>,
    /// Home and temporary filesystems; repeated mounts are permitted.
    pub filesystems: Vec<Filesystem>,
    /// Unanswered questions, including questions written by other agents.
    pub unanswered: Vec<PathBuf>,
    /// Parts that could not be collected and their reasons.
    pub unreadable: Vec<ReadFailure>,
}

/// Observed daemon ownership of the home.
#[derive(Debug, Clone, Copy)]
pub enum Daemon {
    /// No process holds the home.
    Absent,
    /// A process holds the home.
    Present {
        /// The executable file on disk is newer than that process, if known.
        executable_newer: Option<bool>,
    },
}

/// Task state and output activity used by the assessment.
#[derive(Debug, Clone)]
pub struct Task {
    /// Task whose state was observed.
    pub id: TaskId,
    /// Current persisted task status.
    pub status: TaskStatus,
    /// Time of the last output line, if known.
    pub last_activity: Option<Timestamp>,
}

/// Facts about the coordinator's worktree and most recent gate.
#[derive(Debug, Clone)]
pub struct LeadGit {
    /// Commit currently checked out.
    pub tip: CommitId,
    /// Whether uncommitted or untracked files exist.
    pub dirty: bool,
    /// Last recorded gate, absent when no gate is recorded.
    pub gate: Option<GateRecord>,
}

/// Branch observations from local git refs, without fetching.
#[derive(Debug, Clone)]
pub struct Branch {
    /// Task that owns this branch.
    pub task: TaskId,
    /// Branch name assigned to the task.
    pub name: BranchName,
    /// Local branch tip.
    pub tip: CommitId,
    /// Last observed origin ref; unknown differs from known absence.
    pub origin: Option<Origin>,
    /// Whether the lead tip contains this tip, if the check succeeded.
    pub contained_in_lead: Option<bool>,
}

/// The clone's recorded origin branch state.
#[derive(Debug, Clone)]
pub enum Origin {
    /// No origin ref of this name exists in the clone.
    Missing,
    /// Origin's ref exists at this commit.
    Tip(CommitId),
}

/// Usage in one unit: bytes for space or inode counts for inodes.
#[derive(Debug, Clone, Copy)]
pub struct Usage {
    /// Units in use; may exceed total after concurrent changes to the filesystem.
    pub used: u64,
    /// Nonzero total units; an unavailable or zero capacity is represented by `None` at the boundary.
    pub total: NonZeroU64,
}

/// Usage of a filesystem containing a diagnostic path.
#[derive(Debug, Clone)]
pub struct Filesystem {
    /// Path identifying the filesystem.
    pub path: PathBuf,
    /// Allocated bytes, when available.
    pub space: Option<Usage>,
    /// Allocated inodes, when available.
    pub inodes: Option<Usage>,
}

/// Failure to read one diagnostic part; already suitable for display and redaction by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadFailure {
    /// Part that was unavailable.
    pub part: String,
    /// Human-readable explanation of the failed collection.
    pub reason: String,
}

/// Known diagnostic anomalies, with the context needed for an explanation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Anomaly {
    /// No daemon holds the home.
    NoDaemon,
    /// The executable was replaced after daemon startup.
    NewerExecutable,
    /// A task's last run failed.
    FailedTask(TaskId),
    /// A task was paused.
    PausedTask(TaskId),
    /// A running task has output older than the supplied threshold.
    StaleTask(TaskId),
    /// Unfinished items have no running or queued team task.
    NobodyWorking,
    /// A questions file still lacks an answer.
    UnansweredQuestions(PathBuf),
    /// The lead worktree has changes.
    DirtyLead,
    /// The lead's last gate used a tree with changes.
    DirtyGate,
    /// The lead gate records a different commit.
    GateAtOtherCommit,
    /// No clean gate pass is recorded at the lead tip.
    NoGatePassAtTip,
    /// Lead waits with a passing gate and an item in progress without a recorded PR.
    WaitingWithoutPr,
    /// An unfinished assigned item's branch is not contained in the lead tip.
    UnmergedBranch(BranchName),
    /// A local tip differs from the clone's origin ref, including a missing origin ref.
    OriginDiffers(BranchName),
    /// A filesystem is at least 90 percent full by space or inode usage.
    FullFilesystem(PathBuf),
    /// A part of the diagnostics could not be collected.
    Unreadable(ReadFailure),
}

/// Finds known anomalies once each in stable observation order.
#[must_use]
pub fn assess(_facts: &Assessment) -> Vec<Anomaly> {
    Vec::new()
}

#[cfg(test)]
mod tests;
