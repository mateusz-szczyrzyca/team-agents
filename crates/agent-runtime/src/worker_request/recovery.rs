//! Opt-in durable attempt journals for fenced settlement and interruption recovery.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use agent_core::decide::worker_recovery::{Cause, MAX_NO_RESULT_RETURNS};
use agent_core::decide::worker_result::{WorkerEnding, Wrong};
use agent_core::history::SnapshotRecord;
use agent_core::ids::{AgentName, RoundId, RunId, TaskId};
use agent_core::state::CommitId;
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;
use crate::request_workspace::{RequestWorkspace, Stage};

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
    /// A retry starts in a dedicated workspace while preserving the original identity.
    Assigned {
        /// Workspace prepared for this execution.
        workspace: RequestWorkspace,
        /// Fresh execution identifier.
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
    /// The current execution produced a declared ending, retained before settlement.
    Observed {
        /// Execution whose ending was read.
        run: RunId,
        /// Declared result, independent of the check outcome.
        ending: WorkerEnding,
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

/// Untrusted on-disk form. Workspace validation is reused from `request_workspace`.
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
        if record.workspace.record().stage != Stage::Working {
            return Err(Invalid::Transition);
        }
        let mut journal = Self {
            record: Record {
                workspace: record.workspace,
                events: Vec::new(),
            },
            phase: Phase::Queued,
            wrong_try: NonZeroU32::MIN,
            returns: 0,
        };
        let mut runs = BTreeSet::new();
        let mut generation = 0_u32;
        for event in record.events {
            match &event {
                Transition::Assigned { workspace, run } => {
                    let original = journal.workspace().record();
                    let next = workspace.record();
                    if journal.phase != Phase::Queued
                        || !runs.insert(run.clone())
                        || next.request != original.request
                        || next.requester_task != original.requester_task
                        || next.requester_repo != original.requester_repo
                        || next.requester != original.requester
                        || next.stage != Stage::Working
                    {
                        return Err(Invalid::Transition);
                    }
                    generation = generation.checked_add(1).ok_or(Invalid::Transition)?;
                    journal.phase = Phase::Running(Attempt {
                        number: NonZeroU32::new(generation).ok_or(Invalid::Transition)?,
                        worker: next.worker.clone(),
                        run: run.clone(),
                    });
                }
                Transition::Started { worker, run } => {
                    if journal.phase != Phase::Queued || !runs.insert(run.clone()) {
                        return Err(Invalid::Transition);
                    }
                    generation = generation.checked_add(1).ok_or(Invalid::Transition)?;
                    journal.phase = Phase::Running(Attempt {
                        number: NonZeroU32::new(generation).ok_or(Invalid::Transition)?,
                        worker: worker.clone(),
                        run: run.clone(),
                    });
                }
                Transition::Interrupted { run, cause, snapshot } => {
                    journal.require_run(run)?;
                    if *cause == Cause::RestartQueued {
                        return Err(Invalid::Transition);
                    }
                    journal.require_snapshot(snapshot)?;
                    journal.return_without_result();
                }
                Transition::RestartedQueued { snapshot } => {
                    if journal.phase != Phase::Queued {
                        return Err(Invalid::Transition);
                    }
                    journal.require_snapshot(snapshot)?;
                    journal.return_without_result();
                }
                Transition::Rejected { run, wrong, snapshot } => {
                    journal.require_run(run)?;
                    if wrong.is_empty() {
                        return Err(Invalid::Transition);
                    }
                    journal.require_snapshot(snapshot)?;
                    journal.wrong_try = journal.wrong_try.checked_add(1).ok_or(Invalid::Transition)?;
                    journal.phase = Phase::Queued;
                }
                Transition::Observed { run, .. } => {
                    journal.require_run(run)?;
                    if journal.ending().is_some() {
                        return Err(Invalid::Transition);
                    }
                }
                Transition::Settled { run } => {
                    journal.require_run(run)?;
                    journal.phase = Phase::Settled;
                }
            }
            journal.record.events.push(event);
        }
        Ok(journal)
    }
}

impl Journal {
    /// Rejects late answers without changing current ownership.
    fn require_run(&self, run: &RunId) -> Result<(), Invalid> {
        match &self.phase {
            Phase::Running(attempt) if &attempt.run == run => Ok(()),
            Phase::Queued | Phase::Running(_) | Phase::Exhausted | Phase::Settled => Err(Invalid::Transition),
        }
    }

    /// A snapshot must preserve this request's workspace, never the requester's files.
    fn require_snapshot(&self, snapshot: &SnapshotIdentity) -> Result<(), Invalid> {
        if snapshot.task == self.current_workspace().record().worker_task {
            Ok(())
        } else {
            Err(Invalid::Transition)
        }
    }

    /// Keeps the terminal cause without spending a fourth return to the queue.
    fn return_without_result(&mut self) {
        if self.returns >= MAX_NO_RESULT_RETURNS {
            self.phase = Phase::Exhausted;
        } else {
            self.returns = self.returns.saturating_add(1);
            self.phase = Phase::Queued;
        }
    }

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

    /// Ending observed for the current execution, if one was durably recorded.
    #[must_use]
    pub fn ending(&self) -> Option<WorkerEnding> {
        let Phase::Running(attempt) = &self.phase else {
            return None;
        };
        self.record.events.iter().rev().find_map(|event| match event {
            Transition::Observed { run, ending } if run == &attempt.run => Some(*ending),
            _ => None,
        })
    }

    /// Workspace used by the latest execution.
    #[must_use]
    pub fn current_workspace(&self) -> &RequestWorkspace {
        self.record
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                Transition::Assigned { workspace, .. } => Some(workspace),
                _ => None,
            })
            .unwrap_or(&self.record.workspace)
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

/// Maximum serialized journal size in bytes; bounds untrusted reads and retained history.
const MAX_JOURNAL_BYTES: usize = 64 * 1024;

/// Why a durable recovery journal cannot be prepared, loaded or advanced.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The journal cannot be read within its UTF-8 and size bounds.
    #[error("cannot read recovery journal {path:?}")]
    Read {
        /// Journal path.
        path: PathBuf,
        /// Underlying bounded-read failure.
        source: crate::text_file::TextFileError,
    },
    /// The atomic journal write failed.
    #[error("cannot write recovery journal {path:?}")]
    Write {
        /// Journal path.
        path: PathBuf,
        /// Underlying atomic-write failure.
        source: crate::text_file::ReplaceError,
    },
    /// The persisted JSON is not a valid journal.
    #[error("cannot decode recovery journal {path:?}")]
    Decode {
        /// Journal path.
        path: PathBuf,
        /// JSON or replay validation failure.
        source: serde_json::Error,
    },
    /// The validated journal cannot be serialized.
    #[error("cannot encode recovery journal {path:?}")]
    Encode {
        /// Journal path.
        path: PathBuf,
        /// JSON encoding failure.
        source: serde_json::Error,
    },
    /// The proposed transition or original workspace is invalid.
    #[error("cannot advance recovery journal {path:?}")]
    Invalid {
        /// Journal path.
        path: PathBuf,
        /// Validation failure.
        source: Invalid,
    },
    /// A legacy or absent request cannot be implicitly opted into recovery.
    #[error("recovery journal {path:?} is absent")]
    Missing {
        /// Expected journal path.
        path: PathBuf,
    },
    /// The filename's request number differs from the journal's identity.
    #[error("recovery journal {path:?} belongs to request H{found}")]
    Mismatch {
        /// Journal path.
        path: PathBuf,
        /// Request actually recorded.
        found: RequestId,
    },
    /// Appending would exceed the bounded history size.
    #[error("recovery journal {path:?} exceeds {MAX_JOURNAL_BYTES} bytes")]
    TooLarge {
        /// Journal path.
        path: PathBuf,
    },
}

/// Journal location in the original worker task directory, alongside existing request records.
fn journal_path(task_dir: &Path, request: RequestId) -> PathBuf {
    task_dir.join(format!("worker-recovery-{}.json", request.number()))
}

/// Explicitly opts a new request into recovery; never replaces an existing journal.
///
/// This only writes the task's journal, not the requester or worker worktree.
///
/// # Errors
/// [`StoreError::Invalid`] for an ended workspace, or [`StoreError`] for storage failures.
pub async fn prepare(task_dir: &Path, workspace: &RequestWorkspace) -> Result<Journal, StoreError> {
    let path = journal_path(task_dir, workspace.record().request);
    let journal = Journal::new(workspace.clone()).map_err(|source| StoreError::Invalid {
        path: path.clone(),
        source,
    })?;
    let text = serde_json::to_string(&journal).map_err(|source| StoreError::Encode {
        path: path.clone(),
        source,
    })?;
    crate::text_file::create_new_text_file(&path, &text, 0o600)
        .await
        .map_err(|source| StoreError::Write { path, source })?;
    Ok(journal)
}

/// Reloads and replays an explicitly opted-in request; absence preserves legacy handling.
///
/// # Errors
/// [`StoreError`] for invalid JSON, identity mismatches or bounded-read failures.
pub async fn load(task_dir: &Path, request: RequestId) -> Result<Option<Journal>, StoreError> {
    let path = journal_path(task_dir, request);
    let Some(file) = crate::text_file::read_text_file(&path, MAX_JOURNAL_BYTES)
        .await
        .map_err(|source| StoreError::Read {
            path: path.clone(),
            source,
        })?
    else {
        return Ok(None);
    };
    let journal: Journal = serde_json::from_str(&file.text).map_err(|source| StoreError::Decode {
        path: path.clone(),
        source,
    })?;
    if journal.workspace().record().request != request {
        return Err(StoreError::Mismatch {
            path,
            found: journal.workspace().record().request,
        });
    }
    Ok(Some(journal))
}

/// Validates and atomically persists one transition under the daemon's single-writer ownership.
///
/// The caller must serialize writes to this request. A rejected transition leaves its file unchanged.
/// No workspace files are changed. A missing journal never creates recovery for an old request.
///
/// # Errors
/// [`StoreError`] for a missing journal, invalid transition or storage failure.
pub async fn advance(task_dir: &Path, request: RequestId, transition: Transition) -> Result<Journal, StoreError> {
    let path = journal_path(task_dir, request);
    let current = load(task_dir, request)
        .await?
        .ok_or_else(|| StoreError::Missing { path: path.clone() })?;
    let journal = current.advance(transition).map_err(|source| StoreError::Invalid {
        path: path.clone(),
        source,
    })?;
    let text = serde_json::to_string(&journal).map_err(|source| StoreError::Encode {
        path: path.clone(),
        source,
    })?;
    if text.len() > MAX_JOURNAL_BYTES {
        return Err(StoreError::TooLarge { path });
    }
    crate::text_file::replace_text_file(&path, &text, 0o600)
        .await
        .map_err(|source| StoreError::Write { path, source })?;
    Ok(journal)
}

#[cfg(test)]
mod tests;
