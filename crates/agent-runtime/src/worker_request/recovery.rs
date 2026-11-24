//! Opt-in durable attempt journals. No daemon dispatch path uses recovery yet.

use std::num::NonZeroU32;

use agent_core::decide::worker_recovery::Cause;
use agent_core::decide::worker_result::Wrong;
use agent_core::history::SnapshotRecord;
use agent_core::ids::{AgentName, RoundId, RunId, TaskId};
use agent_core::state::CommitId;
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;
use crate::request_workspace::RequestWorkspace;

/// Snapshot identity from the existing snapshot facility; the git reference retains its commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotIdentity {
    /// Round used in the snapshot reference.
    pub round: RoundId,
    /// Task whose worktree was snapshotted.
    pub task: TaskId,
    /// Snapshot reference number, starting at one.
    pub number: NonZeroU32,
    /// Commit preserving the worktree, including uncommitted files.
    pub commit: CommitId,
    /// Parent commit of the snapshot.
    pub head: CommitId,
}

impl TryFrom<&SnapshotRecord> for SnapshotIdentity {
    type Error = Invalid;

    /// Retains the identity of an existing snapshot without taking another one.
    ///
    /// # Errors
    /// [`Invalid::ZeroSnapshot`] for a snapshot numbered zero.
    fn try_from(record: &SnapshotRecord) -> Result<Self, Self::Error> {
        Ok(Self {
            round: record.round.clone(),
            task: record.task.clone(),
            number: NonZeroU32::new(record.number).ok_or(Invalid::ZeroSnapshot)?,
            commit: record.commit.clone(),
            head: record.head.clone(),
        })
    }
}

/// Identity of one execution in the immutable request workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    /// Monotonic generation; not a wrong-result try number.
    pub number: NonZeroU32,
    /// Worker owning this generation.
    pub worker: AgentName,
    /// Globally unique run fencing late results.
    pub run: RunId,
}

/// One durable change, replayed in order to validate the journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transition {
    /// A free worker is about to start in the original workspace.
    Started {
        /// Worker taking ownership.
        worker: AgentName,
        /// Fresh run identifier.
        run: RunId,
    },
    /// A current execution supplied no result.
    Interrupted {
        /// Run whose ownership must still be current.
        run: RunId,
        /// Retained no-result cause.
        cause: Cause,
        /// Snapshot of the original workspace before requeueing.
        snapshot: SnapshotIdentity,
    },
    /// A queued request survived a daemon restart without starting.
    RestartedQueued {
        /// Snapshot retained even when there was no current run.
        snapshot: SnapshotIdentity,
    },
    /// A result was wrong; the next execution spends a wrong-result try.
    Rejected {
        /// Current result's run.
        run: RunId,
        /// Wrong-result reasons, including non-DONE endings after work.
        wrong: Vec<Wrong>,
        /// Snapshot before handing the workspace to the next execution.
        snapshot: SnapshotIdentity,
    },
    /// Settlement finished; subsequent results must be dropped.
    Settled {
        /// Current result's run.
        run: RunId,
    },
}

/// State derived from validated events, never independently deserialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for a compatible free worker.
    Queued,
    /// One execution owns the workspace.
    Running(Attempt),
    /// A fourth missing result exhausted the three allowed returns.
    Exhausted,
    /// The request's result is settled.
    Settled,
}

/// Untrusted on-disk form. Workspace validation is reused from request_workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    /// Original immutable workspace, independent of later workers.
    workspace: RequestWorkspace,
    /// Ordered transitions; counters and ownership are reconstructed from these.
    events: Vec<Transition>,
}

/// A replay-validated journal. Only explicit preparation opts a new request in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Record")]
pub struct Journal {
    /// Durable form; derived state is not trusted from JSON.
    #[serde(flatten)]
    record: Record,
    /// Current ownership or queue state.
    #[serde(skip)]
    phase: Phase,
    /// Wrong-result try, starting at one.
    #[serde(skip)]
    wrong_try: NonZeroU32,
    /// Completed no-result returns, bounded by three.
    #[serde(skip)]
    returns: u8,
}

/// Why an untrusted journal or transition is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Invalid {
    /// A snapshot reference cannot be numbered zero.
    #[error("snapshot number is zero")]
    ZeroSnapshot,
    /// A transition does not follow the current state or owner.
    #[error("transition does not follow the current request state")]
    Transition,
}

impl TryFrom<Record> for Journal {
    type Error = Invalid;

    fn try_from(record: Record) -> Result<Self, Self::Error> {
        Ok(Self {
            record,
            phase: Phase::Queued,
            wrong_try: NonZeroU32::MIN,
            returns: 0,
        })
    }
}

impl Journal {
    /// Prepares an empty journal for a new request in its already validated workspace.
    ///
    /// # Errors
    /// [`Invalid`] for a workspace that has already ended.
    pub fn new(workspace: RequestWorkspace) -> Result<Self, Invalid> {
        Record {
            workspace,
            events: Vec::new(),
        }
        .try_into()
    }

    /// Original workspace identity, unaffected by later worker assignments.
    #[must_use]
    pub fn workspace(&self) -> &RequestWorkspace {
        &self.record.workspace
    }

    /// Validated transition history, including all interruption and wrong-result causes.
    #[must_use]
    pub fn events(&self) -> &[Transition] {
        &self.record.events
    }

    /// Current state reconstructed from the history.
    #[must_use]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Current wrong-result try number.
    #[must_use]
    pub fn wrong_try(&self) -> NonZeroU32 {
        self.wrong_try
    }

    /// No-result returns already used, independently of wrong-result tries.
    #[must_use]
    pub fn returns(&self) -> u8 {
        self.returns
    }

    /// Appends one valid transition, leaving the original unchanged on failure.
    ///
    /// # Errors
    /// [`Invalid`] when ownership, state, identity or the return bound forbids it.
    pub fn advance(&self, transition: Transition) -> Result<Self, Invalid> {
        let mut record = self.record.clone();
        record.events.push(transition);
        record.try_into()
    }
}

#[cfg(test)]
mod tests;
