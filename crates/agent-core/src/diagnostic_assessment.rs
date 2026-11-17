//! Pure assessment of observed team diagnostics; unknown observations never imply a failure.

use std::collections::HashSet;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::time::Duration;

use crate::ids::{BranchName, TaskId};
use crate::state::{CommitId, GateRecord, GateTree, ItemPhase, Timestamp};
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
pub fn assess(facts: &Assessment) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();
    match facts.daemon {
        Some(Daemon::Absent) => anomalies.push(Anomaly::NoDaemon),
        Some(Daemon::Present {
            executable_newer: Some(true),
        }) => anomalies.push(Anomaly::NewerExecutable),
        Some(Daemon::Present {
            executable_newer: Some(false) | None,
        })
        | None => {}
    }
    for task in &facts.tasks {
        match task.status {
            TaskStatus::Failed => anomalies.push(Anomaly::FailedTask(task.id.clone())),
            TaskStatus::Paused => anomalies.push(Anomaly::PausedTask(task.id.clone())),
            TaskStatus::Running => {
                if task.last_activity.is_some_and(|last| {
                    last < facts.now && (facts.now.as_utc() - last.as_utc()).unsigned_abs() > facts.stale_after
                }) {
                    anomalies.push(Anomaly::StaleTask(task.id.clone()));
                }
            }
            TaskStatus::Queued
            | TaskStatus::Waiting
            | TaskStatus::Questions
            | TaskStatus::Stopped
            | TaskStatus::Done => {}
        }
    }
    if facts.items.iter().any(unfinished)
        && !facts
            .tasks
            .iter()
            .any(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Queued))
    {
        anomalies.push(Anomaly::NobodyWorking);
    }
    anomalies.extend(facts.unanswered.iter().cloned().map(Anomaly::UnansweredQuestions));
    assess_lead(facts, &mut anomalies);
    assess_branches(facts, &mut anomalies);
    for filesystem in &facts.filesystems {
        if filesystem.space.is_some_and(full) || filesystem.inodes.is_some_and(full) {
            anomalies.push(Anomaly::FullFilesystem(filesystem.path.clone()));
        }
    }
    anomalies.extend(facts.unreadable.iter().cloned().map(Anomaly::Unreadable));
    // The set owns a second copy so repeated input observations cannot duplicate an explanation.
    let mut seen = HashSet::new();
    anomalies.retain(|anomaly| seen.insert(anomaly.clone()));
    anomalies
}

/// Whether an item is unfinished, exhaustively following the state model.
fn unfinished(item: &ItemPhase) -> bool {
    match item {
        ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. } => true,
        ItemPhase::Done { .. } => false,
    }
}

/// Assesses only the coordinator's observed worktree and recorded gate.
fn assess_lead(facts: &Assessment, anomalies: &mut Vec<Anomaly>) {
    let Some(git) = &facts.lead_git else {
        return;
    };
    if git.dirty {
        anomalies.push(Anomaly::DirtyLead);
    }
    let passed_at_tip = git
        .gate
        .as_ref()
        .is_some_and(|gate| gate.counts_as_pass() && gate.result.head == git.tip);
    if let Some(gate) = &git.gate {
        if gate.tree == GateTree::NotClean {
            anomalies.push(Anomaly::DirtyGate);
        }
        if gate.result.head != git.tip {
            anomalies.push(Anomaly::GateAtOtherCommit);
        }
    }
    if !passed_at_tip {
        anomalies.push(Anomaly::NoGatePassAtTip);
    }
    if passed_at_tip
        && facts
            .tasks
            .iter()
            .any(|task| task.id == facts.lead && task.status == TaskStatus::Waiting)
        && facts
            .items
            .iter()
            .any(|item| matches!(item, ItemPhase::InProgress { .. }))
        && !facts.items.iter().any(|item| {
            matches!(
                item,
                ItemPhase::InReview { .. }
                    | ItemPhase::Fixing { .. }
                    | ItemPhase::Approved { .. }
                    | ItemPhase::Done {
                        completion: crate::state::Completion::Merged { .. }
                    }
            )
        })
    {
        anomalies.push(Anomaly::WaitingWithoutPr);
    }
}

/// Compares only known local observations; assignment is required for a missing integration finding.
fn assess_branches(facts: &Assessment, anomalies: &mut Vec<Anomaly>) {
    let assigned: HashSet<_> = facts
        .items
        .iter()
        .filter_map(|item| match item {
            ItemPhase::InProgress { branch, .. } | ItemPhase::Fixing { branch, .. } => Some(branch),
            ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => {
                None
            }
        })
        .collect();
    for branch in &facts.branches {
        if branch.task != facts.lead && branch.contained_in_lead == Some(false) && assigned.contains(&branch.name) {
            anomalies.push(Anomaly::UnmergedBranch(branch.name.clone()));
        }
        let differs = match &branch.origin {
            Some(Origin::Missing) => true,
            Some(Origin::Tip(tip)) => tip != &branch.tip,
            None => false,
        };
        if differs {
            anomalies.push(Anomaly::OriginDiffers(branch.name.clone()));
        }
    }
}

/// Compares exact fractions without rounding; widening u64 before multiplication prevents overflow.
fn full(usage: Usage) -> bool {
    u128::from(usage.used) * 100 >= u128::from(usage.total.get()) * 90
}

#[cfg(test)]
mod tests;
